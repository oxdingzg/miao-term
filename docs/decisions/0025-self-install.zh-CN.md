# ADR 0025 — 安装并重启(自我替换)

> English (default): [`0025-self-install.md`](0025-self-install.md)

状态:已接受。

## 背景

ADR 0022 已能校验下载的更新,但止步于"打开产物并退出"。把闭环补完意味着替换正在运行的应用
并重启它 —— 这在运行中的进程内部无法完成(其可执行文件/包正被占用)。

## 决定

macOS 上 `install_update` 用一个分离的 helper 完成替换:

1. 要求**已校验**的产物(`update_ready`,仅在摘要与可选签名校验通过后设置)。
2. 若进程不在 `.app` 内(即开发构建),拒绝并只打开产物;`bundle_root()` 识别
   `…/X.app/Contents/MacOS/x`。
3. 解包产物(`unzip`)到暂存目录并找到新的 `.app`(`find_app`)。
4. 写出 `install.sh`:等待我们的 PID 退出 → 把当前包移到 `<bundle>.old` → 把新包就位 →
   `open` 重启 → 删除备份;若移动失败则恢复旧包。
5. 以分离方式启动它(`nohup sh … &`)并退出,故替换发生在我们退出之后。

其它平台该按钮降级为*打开下载文件*加状态提示,因为就地替换(Windows 占用中的 `.exe`、
Linux AppImage/deb)是另一份工作。

## 附记(Windows 与 Linux)

`install_update` 现在三平台都覆盖:用一个分离的 helper 等待我们的 PID 退出,再替换应用:

- **Windows**:`.cmd` 对 MSI 执行 `msiexec /i`(随后启动
  `%ProgramFiles%\miaotty\bin\miaotty.exe`),或对 zip 用 `tar` 解包并把 `miaotty.exe` /
  `miaotty-cli.exe` 覆盖到当前安装位置,然后重启。以 `CREATE_NO_WINDOW` 启动。
- **Linux**:当从 **AppImage** 运行(设置了 `$APPIMAGE`)时,`sh` helper 把下载文件覆盖到该
  AppImage、`chmod +x` 并重新执行;其它格式交给包管理器。

生成器是纯函数,在每个平台都有单测。

## 附记(B4.3,原生应用)

移除 eframe host 后,统一的原生应用(ADR 0032)失去了这一流程;B4.3 在 `mtty-ui/src/install.rs` 中恢复它
(改用 `sh` 单引号转义),`update::download_verified` 改为在程序内(`minisign-verify`)用内置发布公钥校验
minisign 签名,签名为必需项,不再因缺少 `minisign` 工具而跳过。命令面板的 *更新并重启* 一步完成检查、下载、
校验与安装。

## 后果

- macOS 上更新路径端到端可用:检查 → 下载 → 校验 → 安装 → 重启,且替换失败可回滚。
- helper 文本与 `bundle_root` 是纯函数并有单测;真正的替换只能在真实安装的包上验证。
- Windows/Linux 的自我替换与已签名发布通道仍属后续。
- Update:Windows/Linux 的自我替换已落地(见附记);已签名发布通道仍属后续。
