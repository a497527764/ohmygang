; NSIS 安装包的钩子（tauri.conf.json 的 bundle.windows.nsis.installerHooks）。文件要存成带 BOM 的 UTF-8，NSIS 才按 UTF-8 读中文。
; 在 Tauri 模板里，这个文件在 utils.nsh、Win\COM.nsh、FileFunc.nsh 之后引入，可以用它们的宏。
;
; 每次装完（包括自动更新）都告诉 Windows 图标可能变了（3.1.1）：快捷方式指向的一直是同一个 lolgang.exe，
; Windows 按路径缓存了它的图标；Tauri 的模板装完不通知、自动更新时也不重建快捷方式，换了图标后桌面上会一直是旧图。
;
; 3.2.0 删掉了 2.0.1 起从「钢一下」迁移安装的部分（找旧的安装目录、自动更新时改装到旧目录旁边、静默卸掉旧的、补快捷方式），
; 大家都已经是 3.x。删掉也堵住了一条路：以前每次安装都会运行「钢一下」快捷方式指向的目录里的 uninstall.exe，
; 有人放一个伪造的同名快捷方式，安装时就会运行他的程序。

!macro NSIS_HOOK_POSTINSTALL
  ; SHCNE_ASSOCCHANGED（0x08000000）、SHCNF_IDLIST（0）：资源管理器丢掉图标缓存，桌面和开始菜单的快捷方式重新取图标
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend
