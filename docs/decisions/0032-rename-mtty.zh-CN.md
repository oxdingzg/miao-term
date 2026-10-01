# ADR 0032 — 应用正式更名为 mtty

> English (default): [`0032-rename-mtty.md`](0032-rename-mtty.md)

状态:已接受。取代 [APP-IDENTITY](../APP-IDENTITY.zh-CN.md) 中"沿用 miaotty 名称"的结论。

## 背景

产品对外名称确定为 **mtty**(官网 `mtty.dev`)。此前应用、CLI、配置目录、socket、环境变量、
URL scheme、安装包都叫 `miaotty`。已有的用户状态和外部集成依赖旧名字:

- 配置目录 `~/.config/miaotty`(`config.toml`、`views.json`、会话、队列、Recipes、hooks)。
- 已安装的 agent hook 脚本调用 `${MIAOTTY_CLI:-miaotty-cli}`,并读取 `MIAOTTY_PANE_ID`。
- miao 的内置集成读取 `MIAOTTY_PANE_ID` 与 `MIAOTTY_CLI`(缺省时调用 PATH 上的 `miaotty-cli`)。
- 用户粘贴到 skhd / Hammerspoon / sway 等处的热键片段调用 `miaotty --quick`。
- 旧版 CLI 默认连接 `$XDG_RUNTIME_DIR/miaotty.sock`。

## 决定

**新名字是唯一的主名字;旧名字只作为只读兼容输入,并至少保留两个发布版本。**

| 方面 | 新 | 兼容 |
|---|---|---|
| 可执行文件 | `mtty`、`mtty-cli` | Linux 包内提供 `miaotty`、`miaotty-cli` 符号链接 |
| Cargo 包 / 目录 | `mtty-app`、`mtty-cli` | — |
| macOS bundle | `mtty.app`,bundle ID `dev.mtty.terminal` | 安装脚本按身份归档并移除 `miaotty.app`(`io.miaotty.terminal`) |
| 配置目录 | `$XDG_CONFIG_HOME/mtty` | 新目录不存在时,从 `miaotty` 目录**复制**一次,旧目录保留 |
| 环境变量 | `MTTY_*` | 读取时 `MTTY_*` 优先,再读 `MIAOTTY_*`;pane 内**同时导出**两套 |
| CLI 路径 | pane 内 `MTTY_CLI`、`MIAOTTY_CLI` 都设为同目录 `mtty-cli` 的绝对路径 | 旧 hook 与 miao 无需修改即可找到 CLI |
| socket | `$XDG_RUNTIME_DIR/mtty.sock`,Windows 管道 `mtty` | Unix 上另建 `miaotty.sock` 符号链接(不存在或已失效时);新 CLI 找不到 `mtty.sock` 时回退旧路径 |
| URL scheme | `mtty://` | 继续接受并注册 `miaotty://` |
| ssh ControlPath | `~/.ssh/mtty-cm-%r@%h:%p` | 无需兼容(只是新的复用连接) |
| Windows MSI | 产品名 `mtty`,安装目录 `mtty` | **UpgradeCode 不变**,旧版本原地升级 |
| Debian 包 | `mtty` | `Replaces/Conflicts/Provides: miaotty` |

bundle ID 现在就改:项目仍是 0.0.x 预发布,`mtty.dev` 是自有域名,越晚改代价越大。
代价是 macOS 把它视为新应用,通知等权限需重新授予;本 ADR 与发布说明都写明这一点。

迁移集中在 `miao-term-config` 的 `config_dir()` 与 `migrate_legacy_config()`,所有配置/状态路径
都经由它,不再各自拼 `"miaotty"`。环境变量读取集中在 `miao_term_config::env()`(`term-mtp`
无该依赖,使用自身同等的小函数)。

旧 eframe 应用的 `$XDG_DATA_HOME/miaotty/session.json` 仍作为最后一级会话来源,不改名。
历史 ADR 是当时的记录,不改写。

## 后果

- 已有用户升级后配置、会话、Recipes、hooks 原样可用;旧目录保留,可随时回退到旧版本。
- miao、旧 hook、旧热键片段(Linux)与旧 CLI 在过渡期继续工作。
- 两个版本后(预计 v0.0.8)再评估移除 `MIAOTTY_*` 导出与旧符号链接,届时另立 ADR。
- 迁移逻辑有单元测试(复制、不覆盖、不删除旧目录、新目录已存在时不动作)。
