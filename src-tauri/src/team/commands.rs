//! 队伍命令和个人资料：都在节点任务里执行，完了返回整份队伍页。见 `team` 文件头第 1 条。

use tauri::{AppHandle, Manager};

use super::view::{AvatarInput, RecentTeamDto, TeamView, recent_dto, unbase64, view};
use super::{refresh_names, refresh_presence};
use crate::api::AppState;
use crate::app_core::{AppError, Core};
use crate::crypto::RoomId;
use crate::node::{Outbox, now_ms};
use crate::protocol::body::Avatar;
use crate::protocol::live::LiveBody;

/// 在节点任务里改队伍，再返回整份队伍页。
async fn act<F>(app: &AppHandle, f: F) -> Result<TeamView, String>
where
    F: FnOnce(&mut Core, &mut Outbox) -> Result<(), AppError> + Send + 'static,
{
    let node = app.state::<AppState>().node.clone();
    node.call(f)
        .await
        .ok_or("网络模块停了")?
        .map_err(|e| e.to_string())?;
    refresh_names(app);
    refresh_presence(app);
    view(app).await
}

fn current(core: &Core) -> Result<RoomId, AppError> {
    core.current_team()?
        .map(|t| t.room_id)
        .ok_or(AppError::RoomNotFound)
}

#[tauri::command]
pub async fn team_view(app: AppHandle) -> Result<TeamView, String> {
    view(&app).await
}

/// 主页「加入过的队伍」：按最近活跃倒序，不含当前队伍和已解散的。点击一行直接 `team_join` 它的 code。
#[tauri::command]
pub async fn team_recent(app: AppHandle) -> Result<Vec<RecentTeamDto>, String> {
    let node = app.state::<AppState>().node.clone();
    let list = node
        .call(|core, _| core.recent_rooms())
        .await
        .ok_or("网络模块停了")?
        .map_err(|e| e.to_string())?;
    Ok(list.iter().map(recent_dto).collect())
}

/// 启动自动回归：当时没在队里、最近一个队伍不是已解散的，就重新 join 它。
/// 失败（队伍码坏了、网络没起来）不打扰用户：队伍列表里那一行还在，手动点就是。
pub async fn rejoin_last(app: &AppHandle) {
    let node = app.state::<AppState>().node.clone();
    let joined = node
        .call(
            |core: &mut Core, _: &mut Outbox| -> Result<bool, AppError> {
                if core.current_team()?.is_some() {
                    return Ok(false);
                }
                let Some(last) = core.recent_rooms()?.into_iter().next() else {
                    return Ok(false);
                };
                core.join_room(&last.code, now_ms())?;
                Ok(true)
            },
        )
        .await;
    if matches!(joined, Some(Ok(true))) {
        super::refresh_names(app);
    }
}

/// 建队：先向云端申请 6 位短码（服务器铸造 GT2 长码），再本地建队、写事件、立刻报心跳。
#[tauri::command]
pub async fn team_create(app: AppHandle, title: String) -> Result<TeamView, String> {
    let node = app.state::<AppState>().node.clone();
    let member_hex = node
        .call(|core, _| crate::crypto::hex(&core.identity().member_id))
        .await
        .ok_or("网络模块停了")?;
    let created = crate::cloud::team_create(&member_hex).await?;
    let (code, short) = (created.code, created.short_code);
    let view = act(&app, move |core, out| {
        let c = core.create_room_with_code(&code, &title, now_ms())?;
        core.set_short_code(&c.summary.room_id, &short)?;
        out.broadcast(c.summary.room_id, c.events);
        Ok(())
    })
    .await?;
    kick_heartbeat(&app);
    Ok(view)
}

/// 进队：输入 6 位短码就向云端换 GT2 长码；GT2 长码直接用（兼容存量队伍）。
#[tauri::command]
pub async fn team_join(app: AppHandle, code: String) -> Result<TeamView, String> {
    let (gt2, short) = resolve_code(code).await?;
    let view = act(&app, move |core, _| {
        let joined = core.join_room(&gt2, now_ms())?;
        if let Some(s) = short {
            core.set_short_code(&joined.room_id, &s)?;
        }
        Ok(())
    })
    .await?;
    kick_heartbeat(&app);
    Ok(view)
}

/// 6 位短码 → 向云端换 GT2 长码；GT2 长码原样返回。
async fn resolve_code(input: String) -> Result<(String, Option<String>), String> {
    let input = input.trim().to_string();
    if input.to_ascii_uppercase().starts_with("GT2") {
        return Ok((input, None));
    }
    let normalized: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if normalized.chars().count() == 6 {
        let joined = crate::cloud::team_join(&normalized).await?;
        Ok((joined.code, Some(normalized)))
    } else {
        Err("队伍码应为 6 位字母数字（或 GT2- 开头的长码）".to_string())
    }
}

/// 建队/进队后立刻给活跃队伍报一次心跳。
fn kick_heartbeat(app: &AppHandle) {
    let node = app.state::<AppState>().node.clone();
    tauri::async_runtime::spawn(async move { crate::cloud::heartbeat_all(&node).await });
}

/// 正在加入时取消。
#[tauri::command]
pub async fn team_cancel(app: AppHandle) -> Result<TeamView, String> {
    act(&app, |core, _| {
        let room = current(core)?;
        core.cancel_join(&room)
    })
    .await
}

/// 退出：先发 Bye，再写 Leave。
#[tauri::command]
pub async fn team_leave(app: AppHandle) -> Result<TeamView, String> {
    let view = act(&app, |core, out| {
        let room = current(core)?;
        out.live(room, LiveBody::Bye);
        let written = core.leave_room(&room, now_ms())?;
        out.broadcast(room, written);
        Ok(())
    })
    .await;
    app.state::<AppState>()
        .team
        .presence
        .lock()
        .expect("在线状态锁没有中毒")
        .clear();
    view
}

#[tauri::command]
pub async fn team_dissolve(app: AppHandle) -> Result<TeamView, String> {
    act(&app, |core, out| {
        let room = current(core)?;
        let written = core.dissolve_room(&room, now_ms())?;
        out.broadcast(room, written);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn team_rename(app: AppHandle, title: String) -> Result<TeamView, String> {
    act(&app, move |core, out| {
        let room = current(core)?;
        let written = core.rename_room(&room, &title, now_ms())?;
        out.broadcast(room, written);
        Ok(())
    })
    .await
}

/// 改个人资料：昵称、内置头像、200×200 大头像（base64，界面已经裁好压好）。
/// 大头像先导进数据目录（验文件头和大小）拿到哈希，再进事务；`avatar_image` 为 None 表示不用大图（换回内置头像）。
#[tauri::command]
pub async fn profile_set(
    app: AppHandle,
    nickname: String,
    avatar: Option<AvatarInput>,
    avatar_image: Option<String>,
) -> Result<TeamView, String> {
    let avatar = match avatar {
        None => None,
        Some(AvatarInput::Preset { icon, color }) => Some(Avatar::Preset { icon, color }),
        Some(AvatarInput::Image { data }) => Some(Avatar::Image(unbase64(&data)?)),
    };
    let avatar_hash = match avatar_image {
        Some(data) => {
            let bytes = unbase64(&data)?;
            let store = app.state::<AppState>().emotes.clone();
            Some(store.import_avatar(&bytes).map_err(|e| e.to_string())?)
        }
        None => None,
    };
    // 不走 act：这里要先把新哈希写进缓存、再刷在线状态（act 的顺序做不到）
    let new_hash = avatar_hash.clone();
    let node = app.state::<AppState>().node.clone();
    node.call(move |core, out| {
        let written =
            core.set_profile(nickname.trim(), avatar, avatar_hash.as_deref(), now_ms())?;
        if let Some(room) = core.current_team()?.map(|t| t.room_id) {
            out.broadcast(room, written);
        }
        Ok(())
    })
    .await
    .ok_or("网络模块停了")?
    .map_err(|e: AppError| e.to_string())?;
    if let Some(state) = app.try_state::<AppState>() {
        *state.team.avatar_hash.lock().expect("头像哈希锁没有中毒") = new_hash;
    }
    refresh_names(&app);
    refresh_presence(&app);
    view(&app).await
}
