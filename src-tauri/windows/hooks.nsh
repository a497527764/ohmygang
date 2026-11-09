; NSIS 安装包的钩子（tauri.conf.json 的 bundle.windows.nsis.installerHooks）。文件要存成带 BOM 的 UTF-8，NSIS 才按 UTF-8 读中文。
; 在 Tauri 模板里，这个文件在 utils.nsh、Win\COM.nsh、FileFunc.nsh 之后引入，可以用它们的宏。
;
; 2.0.0 及以前产品名是「钢一下」，2.0.1 起叫 lolGang。从「钢一下」升上来时：
; 1. 找旧的安装目录：先看开始菜单、桌面上「钢一下」快捷方式指向哪里（用户看得见的那份），再看注册表里记的安装位置；
;    目录里要有 lolgang.exe 和 uninstall.exe 才算。注册表不可靠：有的机器上 2.0.0 的安装记录是空的。
; 2. 自动更新（/UPDATE）时还没有 lolGang 的安装记录，安装包会用默认目录 %LOCALAPPDATA%\lolGang；
;    改成装在旧目录旁边（比如 D:\Program Files\钢一下 → D:\Program Files\lolGang）。手动安装时用户自己选目录，不改。
; 3. 装好后静默卸掉旧的（数据目录按 com.lijia.lolgang 存，两者共用；静默卸载不勾「删除数据」，数据留着），
;    再补上开始菜单快捷方式（/UPDATE 模式下安装包本身不建快捷方式，旧的又随卸载删了）；旧的有桌面快捷方式时，桌面也补一个。
;
; 每次装完（包括自动更新）都告诉 Windows 图标可能变了（3.1.1）：快捷方式指向的一直是同一个 lolgang.exe，
; Windows 按路径缓存了它的图标；Tauri 的模板装完不通知、自动更新时也不重建快捷方式，换了图标后桌面上会一直是旧图。

!define /ifndef MAX_PATH 260
!define /ifndef SLGP_RAWPATH 0x4
!define /ifndef STGM_READ 0

; 快捷方式指向某个目录里的 lolgang.exe、而且那个目录里有 uninstall.exe：把目录放进 $R8（$R8 已经有值就不看）。
!macro LOLGANG_OLD_FROM_SHORTCUT shortcut
  ${If} $R8 == ""
  ${AndIf} ${FileExists} "${shortcut}"
    Push $0
    Push $1
    Push $2
    !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
    ${If} $0 P<> 0
      ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}", .r1)'
      ${If} $1 P<> 0
        ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READ})'
        ${IShellLink::GetPath} $0 '(.r2, ${MAX_PATH}, 0, ${SLGP_RAWPATH})'
        ${GetFileName} $2 $R6
        ${If} $R6 == "lolgang.exe"
          ${GetParent} $2 $R6
          ${If} ${FileExists} "$R6\uninstall.exe"
            StrCpy $R8 $R6
          ${EndIf}
        ${EndIf}
        ${IUnknown::Release} $1 ""
      ${EndIf}
      ${IUnknown::Release} $0 ""
    ${EndIf}
    Pop $2
    Pop $1
    Pop $0
  ${EndIf}
!macroend

; 旧的「钢一下」安装目录放进 $R8，没有就是空的；装在 $INSTDIR 本身的不算。
!macro LOLGANG_FIND_OLD
  StrCpy $R8 ""
  !insertmacro LOLGANG_OLD_FROM_SHORTCUT "$SMPROGRAMS\钢一下.lnk"
  !insertmacro LOLGANG_OLD_FROM_SHORTCUT "$DESKTOP\钢一下.lnk"
  ${If} $R8 == ""
    ReadRegStr $R6 HKCU "Software\lijianqiao\钢一下" ""
    ${If} $R6 != ""
    ${AndIf} ${FileExists} "$R6\lolgang.exe"
    ${AndIf} ${FileExists} "$R6\uninstall.exe"
      StrCpy $R8 $R6
    ${EndIf}
  ${EndIf}
  ${If} $R8 == $INSTDIR
    StrCpy $R8 ""
  ${EndIf}
!macroend

; 这时 SetOutPath 已经建了默认目录（还是空的），删掉它再换目录。
!macro NSIS_HOOK_PREINSTALL
  !insertmacro LOLGANG_FIND_OLD
  ReadRegStr $R7 HKCU "Software\lijianqiao\${PRODUCTNAME}" ""
  ${If} $UpdateMode = 1
  ${AndIf} $R8 != ""
  ${AndIf} $R7 == ""
    ${GetParent} $R8 $R7
    RMDir $INSTDIR
    StrCpy $INSTDIR "$R7\${PRODUCTNAME}"
    SetOutPath $INSTDIR
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro LOLGANG_FIND_OLD
  ${If} $R8 != ""
    StrCpy $R9 0
    ${If} ${FileExists} "$DESKTOP\钢一下.lnk"
      StrCpy $R9 1
    ${EndIf}
    ; _?= 让卸载程序就地运行，并等它结束；它不会删自己，所以之后手动删掉
    ExecWait '"$R8\uninstall.exe" /S _?=$R8'
    Delete "$R8\uninstall.exe"
    RMDir "$R8"
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    ${If} $R9 = 1
      CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    ${EndIf}
  ${EndIf}
  ; SHCNE_ASSOCCHANGED（0x08000000）、SHCNF_IDLIST（0）：资源管理器丢掉图标缓存，桌面和开始菜单的快捷方式重新取图标
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend
