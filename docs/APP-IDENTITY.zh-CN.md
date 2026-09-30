# 统一应用：miaotty

[English](APP-IDENTITY.md)

## 名称与实现

应用名 **miaotty**，主命令 `miaotty`，配套 CLI 为 `miaotty-cli`。
仓库和可嵌入终端引擎继续叫 **miao-term**。
`miaoterm` 更直观地表达 terminal，但没有实现上的收益，还需要改动 CLI、URL scheme、
配置和安装包身份，因此采用已有的 miaotty 名称。

`miaotty-app/src/main.rs` 现在直接启动 `miao-term-widget` 原生实现。
旧 eframe 应用和独立 `miaotty-native` 二进制退役，取代 ADR 0030/0031 描述的双 host 阶段。
`miaotty --version` 输出 `miaotty <版本> (native)`，让校验可以确认实际实现，而非根据名字猜测。

每份正式发布产物只包含一个 GUI 应用及其 CLI：

- macOS：一个 `miaotty.app`，执行文件 `miaotty`，bundle ID 保留 `io.miaotty.terminal`。
- Linux：tar/DEB/AppImage 中只有 `miaotty` 和 `miaotty-cli`。
- Windows：zip/MSI 中只有 `miaotty.exe` 和 `miaotty-cli.exe`。

当前发布配置不再包含 `miaotty-native.app`、独立 native 压缩包或第二个 GUI 二进制。
四个 runner 的构建和打包仍是发布门禁，本地 macOS 校验不替代 Linux/Windows 门禁。

## 已有配置与会话

配置目录、CLI 协议、socket 和 URL scheme 保持兼容。
统一的 native 应用在 `$XDG_CONFIG_HOME/miaotty`（默认 `~/.config/miaotty`）写入
`session.json`、`queue.json`、`window`。读取优先级：

1. 配置目录中的新统一状态文件。
2. 原 native 的 `native-session.json`、`native-queue.json`、`native-window`。
3. 没有 native 会话时，读取原 eframe 的 `$XDG_DATA_HOME/miaotty/session.json`
   （默认 `~/.local/share/miaotty/session.json`）。

旧文件保留。eframe 的索引式布局和焦点转换为 native pane ID，保留分屏方向/比例、cwd、
标题、前缀、标记、分组、活动标签和最近文件；Recipes 也经过同样转换。
两套旧应用都有会话时优先恢复 native，旧 eframe 会话留在原处，不擅自合并。

恢复的是布局并创建新 shell，不是保住仍在运行的进程。配置导入及 Recipes 的位置保持兼容。

## 构建、安装与核验

```sh
cargo build --release -p miaotty-app -p miaotty-cli
scripts/package-macos.sh
python3 scripts/check-macos-bundle.py dist/miaotty.app
python3 scripts/smoke-hosts.py --bundle dist/miaotty.app
bash scripts/install-macos.sh
```

macOS 安装脚本替换身份匹配的 `miaotty.app`，移除身份匹配的旧 `miaotty-native.app`，
并把旧 app 压缩备份到本地临时目录。配置保留，也不会停止正在运行的会话。
若旧应用仍在运行，需要完全退出后重新打开 miaotty，才能切换当前进程。

桌面冒烟检查的是**实际打包的主程序和包内 CLI**，包含原 native 分屏会话迁移、
pane 聚焦/关闭、真实 shell 命令、文件读写、历史、Agent 状态、view/edit 派发及新鲜 GPU 截图。
`MIAOTTY_SHOT_AFTER=<秒数>` 写 `/tmp/miaotty_shot.ppm` 并退出；旧 native 截图环境变量保留为兼容别名。

`UI-AUDIT.zh-CN.md` 中的双 host 审计属于历史证据；当前冒烟脚本只验证统一的 native miaotty。
应用保留 native 已有的版本检查界面；旧 eframe 的自动更新下载/安装界面不宣称为当前应用功能。
