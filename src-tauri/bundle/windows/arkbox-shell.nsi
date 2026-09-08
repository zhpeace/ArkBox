# ArkBox Windows 右键菜单注册表片段（NSIS）
# 用法：放进 Tauri NSIS 安装脚本的 install 段（或 bundle.windows.nsis.template 的对应位置）。
# 前提：bz-cli.exe 已被打包到 $INSTDIR（与主程序 ArkBox.exe 同级）。
# 卸载段用对应的 DeleteRegKey/DeleteRegValue 即可（删 app 时右键项自动消失，满足「卸载即清」）。

!macro ArkBoxShellInstall
  # 压缩
  WriteRegStr HKCR "*\shell\ArkBoxCompress" "" "用 ArkBox 压缩"
  WriteRegStr HKCR "*\shell\ArkBoxCompress\command" "" '"$INSTDIR\bz-cli.exe" compress "%1"'
  # 解压（仅在压缩包上出现）
  WriteRegStr HKCR "*\shell\ArkBoxExtract" "" "用 ArkBox 解压"
  WriteRegStr HKCR "*\shell\ArkBoxExtract\command" "" '"$INSTDIR\bz-cli.exe" extract "%1"'
!macroend

!macro ArkBoxShellUninstall
  DeleteRegKey HKCR "*\shell\ArkBoxCompress"
  DeleteRegKey HKCR "*\shell\ArkBoxExtract"
!macroend
