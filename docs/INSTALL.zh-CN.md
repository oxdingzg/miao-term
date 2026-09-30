# 安装 / 构建

> English (default): [`INSTALL.md`](INSTALL.md)

## 从源码运行

```sh
cargo run -p miaotty-app      # 或:cargo build && ./target/debug/miaotty
```

需要 Rust stable(见 `rust-toolchain.toml`)。首次构建会编译 wgpu/glyphon,可能几分钟。

## macOS .app

```sh
scripts/package-macos.sh              # release 构建 → dist/miaotty.app + dist/miaotty-native.app
PROFILE=debug scripts/package-macos.sh
```

产出 ad-hoc 签名的 `dist/miaotty.app`(eframe 版)与 `dist/miaotty-native.app`(原生版);
两个 bundle 都内含两个二进制、URL scheme 与应用图标。配置 Apple secrets 后,分发时会自动 codesign +
公证(见[发布](#发布))。

## 发布

推 `v*` tag 会触发 [`.github/workflows/release.yml`](../.github/workflows/release.yml):
在 macOS/Linux/Windows 构建 `miaotty` + `miaotty-cli` + `miaotty-native`,并把
`miaotty.app` zip / Linux tar / Windows zip
挂到 GitHub Release。

macOS 默认 ad-hoc 签名;若仓库配了 `APPLE_CERT_P12`+`APPLE_CERT_PASSWORD`+`APPLE_ID`+
`APPLE_TEAM_ID`+`APPLE_APP_PASSWORD` 这些 secrets,则改为 Developer ID 签名 + 公证 + staple。
Linux 额外产出 `.deb`(用 `cargo-deb`,元数据在 `miaotty-app/Cargo.toml`),
并产出 AppImage(`appimagetool`);Windows 为 zip 与 MSI(`cargo-wix`/WiX)。
所有包均包含 `miaotty`、`miaotty-cli` 与 `miaotty-native`;AppImage/MSI 打包失败会使工作流失败。
若存在 `WINDOWS_CERT_PFX` + `WINDOWS_CERT_PASSWORD` secrets,则用 `signtool`
签 MSI。

用 `gh workflow run release.yml --ref main -f tag=v0.0.1` 执行打包演练。
它上传 `release-assembled`(安装包、公钥与 `latest.json`),不创建 GitHub Release。
检查内容及剩余桌面验收见 [`RELEASE.zh-CN.md`](RELEASE.zh-CN.md)。

## 跨平台安装包

`dist-workspace.toml` 是 [cargo-dist](https://opensource.axo.dev/cargo-dist/) 脚手架
(shell/PowerShell 安装器 + MSI)。发布时跑 `dist init` 再 `dist build` 生成各平台产物。

## 配置

- `~/.config/miaotty/config.toml`(或 `$XDG_CONFIG_HOME/miaotty/config.toml`)——
  见 [`config.example.toml`](config.example.toml)。
- 若没有 miaotty 配置,会自动导入 ghostty `config` 与 alacritty `alacritty.toml`。

## 深链接(URL scheme)

这两个 macOS bundle 会向系统注册 `miaotty://`、`ssh://` 与 `x-man-page://`;Linux 的 `.desktop`
文件注册同样三个。链接会被翻译成一个运行对应命令的新标签。

由于两个 host 共用同一个控制 socket 以及其旁的 inbox,只要**装过**这个 bundle 即可:当链接到达
而任意 host(eframe 版或 `miaotty-native`)正在运行时,启动进程会把 intent 转发给它并退出,
由正在运行的窗口处理该深链接。这也是 `miaotty-native` 自身无须注册就能接收系统链接的方式;
bundle 里同时附带两个二进制(`Contents/MacOS/miaotty` 与 `miaotty-native`)。

Windows MSI 只注册 `miaotty://`,故意不接管 `ssh://` / `x-man-page://`,避免在全机器范围抢占。

## Shell 集成

miaotty 会安装 zsh 的 `ZDOTDIR` shim,让 shell 上报 cwd(OSC 7)与命令历史;用户 dotfiles 不受影响,
无需手动配置。
