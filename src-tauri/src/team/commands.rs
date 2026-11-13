//! 队伍命令和个人资料：都在节点任务里执行，完了返回整份队伍页。见 `team` 文件头第 1 条。

use tauri::{AppHandle, Manager};

use super::view::{AvatarInput, RecentTeams, TeamView, recent_dto, unbase64, view};
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
pub async fn team_recent(app: AppHandle) -> Result<RecentTeams, String> {
    let node = app.state::<AppState>().node.clone();
    // 一次 node.call 拿全（列表 + 当前队伍）：分两次调会排两次队，中间队伍状态可能变，
    // 前后两份数据对不上，界面上的「当前队伍」和历史会打架。
    let (list, current) = node
        .call(|core, _| {
            let current = core.current_team()?.map(|t| t.room_id);
            Ok::<_, AppError>((core.recent_rooms()?, current))
        })
        .await
        .ok_or("网络模块停了")?
        .map_err(|e| e.to_string())?;
    // 顺手问云端每个队伍现在几个人在线（15 分钟内报过心跳），以及短码还在不在。
    // 问不到（服务器暂时连不上）不报错：那一列显示「—」就行，别把整个列表拖死。
    let mut ids: Vec<String> = list
        .iter()
        .map(|r| crate::crypto::hex(&r.room_id))
        .collect();
    if let Some(r) = current {
        ids.push(crate::crypto::hex(&r));
    }
    let stats = crate::cloud::team_stats(&ids).await.ok();
    let online = stats.as_ref().map(|s| &s.online);
    let codes = stats.as_ref().map(|s| &s.codes);
    let rooms = list
        .iter()
        .map(|r| {
            let mut dto = recent_dto(r);
            let key = crate::crypto::hex(&r.room_id);
            dto.online = online.and_then(|m| m.get(&key).copied());
            // 本地存着短码，云端却说这房间的码已经没了 → 码到期被回收了。
            // 提前告诉用户，别让他拿着一个别人已经进不来的码去分享。
            if dto.short_code.is_some() && codes.and_then(|m| m.get(&key)) == Some(&None) {
                dto.code_expired = true;
            }
            dto
        })
        .collect();
    let current_online =
        current.and_then(|r| online.and_then(|m| m.get(&crate::crypto::hex(&r))).copied());
    Ok(RecentTeams {
        rooms,
        current_online,
    })
}

/// 启动自动回归：当时没在队里、最近一个队伍不是已解散的，就重新 join 它。
/// 失败（队伍码坏了、网络没起来）不打扰用户：队伍列表里那一行还在，手动点就是。
pub async fn rejoin_last(app: &AppHandle) {
    let node = app.state::<AppState>().node.clone();
    let joined = node
        .call(
            |core: &mut Core, out: &mut Outbox| -> Result<bool, AppError> {
                if core.current_team()?.is_some() {
                    return Ok(false);
                }
                let Some(last) = core.recent_rooms()?.into_iter().next() else {
                    return Ok(false);
                };
                let (joined, written) = core.join_room(&last.code, now_ms())?;
                out.broadcast(joined.room_id, written);
                Ok(true)
            },
        )
        .await;
    if matches!(joined, Some(Ok(true))) {
        super::refresh_names(app);
    }
}

/// 建队：先向云端申请 6 位短码（服务器铸造 GT2 长码），再本地建队、写事件、立刻报心跳。
///
/// 云端拒绝（额度用尽、被限流）或连不上时，退回本地铸一个 GT2 长码：
/// 队伍照样建得起来、队友照样进得来，代价只是没有 6 位短码。
/// 云端配额是保护服务器的，不该让用户连队都建不了。
#[tauri::command]
pub async fn team_create(app: AppHandle, title: String) -> Result<TeamView, String> {
    let node = app.state::<AppState>().node.clone();
    let member_hex = node
        .call(|core, _| crate::crypto::hex(&core.identity().member_id))
        .await
        .ok_or("网络模块停了")?;
    let created = crate::cloud::team_create(&member_hex, &title).await;
    let (view, notice) = match created {
        Ok(c) => {
            let (code, short) = (c.code, c.short_code);
            let v = act(&app, move |core, out| {
                let c = core.create_room_with_code(&code, &title, now_ms())?;
                core.set_short_code(&c.summary.room_id, &short)?;
                out.broadcast(c.summary.room_id, c.events);
                Ok(())
            })
            .await?;
            (v, None)
        }
        Err(e) => {
            eprintln!("云端没给短码，本地铸码兜底：{e}");
            let v = act(&app, move |core, out| {
                let c = core.create_room(&title, now_ms())?;
                out.broadcast(c.summary.room_id, c.events);
                Ok(())
            })
            .await?;
            (
                v,
                Some("云端暂时不给短码了（额度用尽或连不上），这次用长码分享".to_string()),
            )
        }
    };
    let mut view = view;
    if let Some(t) = view.team.as_mut()
        && let Some(n) = notice
    {
        t.notice = Some(n);
    }
    kick_heartbeat(&app);
    Ok(view)
}

/// 进队：输入 6 位短码就向云端换 GT2 长码；GT2 长码直接用（兼容存量队伍）。
#[tauri::command]
pub async fn team_join(app: AppHandle, code: String) -> Result<TeamView, String> {
    let (gt2, short, cloud_title) = resolve_code(code).await?;
    let view = act(&app, move |core, out| {
        let (joined, written) = core.join_room(&gt2, now_ms())?;
        if let Some(s) = short {
            core.set_short_code(&joined.room_id, &s)?;
        }
        // 服务器上存的队名：房主的建队事件收不到时（中继重启过）用它兜底。
        if let Some(t) = cloud_title {
            core.set_local_title(&joined.room_id, &t)?;
        }
        // 自己的入队事件马上发出去：队友在线的话几秒内就能看到我
        out.broadcast(joined.room_id, written);
        Ok(())
    })
    .await?;
    kick_heartbeat(&app);
    Ok(view)
}

/// 房主把某个队员踢出队伍（block=false）或拉黑（block=true）。
/// 拉黑之后这个人再发进队事件也不算数（所有成员都按房主的事件推导成员名单）。
#[tauri::command]
pub async fn team_moderate(
    app: AppHandle,
    member_id: String,
    block: bool,
) -> Result<TeamView, String> {
    let bytes = crate::crypto::parse_hex(&member_id).ok_or("队员标识不对")?;
    let target: crate::crypto::MemberId =
        bytes.try_into().map_err(|_| "队员标识不对".to_string())?;
    let view = act(&app, move |core, out| {
        let written = core.moderate_member(target, block, now_ms())?;
        if let Some(room) = core.current_team()?.map(|t| t.room_id) {
            out.broadcast(room, written);
        }
        Ok(())
    })
    .await?;
    refresh_names(&app);
    Ok(view)
}

/// 从本机历史里删掉一个队伍（主页「加入过的队伍」右键 → 删除这条记录）。
/// 只删本机记录：不写事件、不影响队友，队伍本身还在，别人照样能用队伍码进来。
#[tauri::command]
pub async fn team_forget(app: AppHandle, room_id: String) -> Result<RecentTeams, String> {
    let node = app.state::<AppState>().node.clone();
    let bytes = crate::crypto::parse_hex(&room_id).ok_or("队伍标识不对")?;
    let room: crate::crypto::RoomId = bytes.try_into().map_err(|_| "队伍标识不对".to_string())?;
    node.call(move |core, _| core.forget_room(&room))
        .await
        .ok_or("网络模块停了")?
        .map_err(|e| e.to_string())?;
    team_recent(app).await
}

/// 6 位短码 → 向云端换 GT2 长码和队名；GT2 长码原样返回。
async fn resolve_code(input: String) -> Result<(String, Option<String>, Option<String>), String> {
    let input = input.trim().to_string();
    if input.to_ascii_uppercase().starts_with("GT2") {
        return Ok((input, None, None));
    }
    let normalized: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if normalized.chars().count() == 6 {
        let joined = crate::cloud::team_join(&normalized).await?;
        Ok((joined.code, Some(normalized), joined.title))
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
    crate::api::backup_identity(&app);
    view(&app).await
}
