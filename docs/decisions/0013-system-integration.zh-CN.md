# ADR 0013 — URL scheme、快速终端、i18n、更新检查

> English (default): [`0013-system-integration.md`](0013-system-integration.md)

状态:已接受。

## 背景

要成为*可用*的终端而非演示品,miaotty 必须接入操作系统:打开 `ssh://` 与
`x-man-page://` 链接、提供临时终端、支持英语之外的语言,并在有更新时告知 —— 且不引入
重型依赖。

## 决定

**URL scheme**(`crates/term-ui/src/launch.rs`)。可执行文件从 argv 中找出 `scheme://…`
并转换为新标签里的 shell 命令:`ssh://[user@]host[:port][/path]` →
`ssh [-p port] host`(IPv6 用 `[..]`,参数单引号包裹),`x-man-page://cmd` → `man cmd`,
`miaotty://…` → 仅激活。注册信息位于 macOS `Info.plist`(由
`scripts/package-macos.sh` 生成)与 `cargo-deb` 携带的 Linux `.desktop` 文件;Windows
注册仅文档化,不自动。

**快速终端。** ⌘⇧T(或面板动词)切换一个临时标签:首次使用时创建,之后在它与上一个活动
标签间切换。真正的全局热键需要平台专用 API,推迟。

**i18n**(`crates/term-ui/src/i18n.rs`)。以英文为键的字符串表:`En` 直接返回键,`Zh` 映射
可见界面(设置、Details 标签、面板动词、编辑器/Composer/配方按钮、分区标题、agent 闭环
文案)。由配置 `language` 或 `$LANG` 选择;未翻译项回退到键,即英文。

**更新检查。** 设置 `update-check-url` 后,面板动词或设置按钮在线程里用 `curl`
(`--max-time 5`)拉取该 URL,把首个 token 与 `CARGO_PKG_VERSION` 比较,报告*已是最新*/ 
*有新版本*。启动时还会静默检查一次(配置 `update-auto-check`,默认开启;见 ADR 0022),
有新版本时在状态栏提示。不新增 TLS/运行时依赖。

## 后果

- 集成停在 shell 层且无依赖,故三平台皆可用,且不会导致启动失败。
- 翻译是增量式的:加一门语言只需一个 match 分支;未翻译的键会显示为英文而非空白。
- 真正的全局热键、深链接聚焦已有窗口、应用内更新下载/签名仍属后续。
- Update:真正的全局热键与深链接聚焦已在 ADR 0019 落地(Wayland 在 ADR 0026),
  应用内下载、校验与安装已在 ADR 0022/0025 落地。
