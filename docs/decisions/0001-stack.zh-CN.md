# ADR 0001 — 技术栈选型

> English (default): [`0001-stack.md`](0001-stack.md)

状态:已接受。

## 背景

需要一套跨平台(macOS/Linux/Windows)终端。先例:Alacritty、WezTerm、Rio(均 Rust),
Ghostty(Zig + 平台 UI)。自研 VT 内核昂贵且不是差异点;差异在渲染与应用功能。

## 决定

- **PTY**:`portable-pty`(MIT;Unix `forkpty`,Windows **ConPTY**)。
- **VT / 网格 / 回滚**:`alacritty_terminal`(Apache-2.0)+ `vte`(Apache-2.0)。
- **窗口/输入**:`winit`。
- **GPU**:`wgpu`(Metal / Vulkan / DX12)。
- **字形渲染**:基于 wgpu 自绘网格,用 `glyphon` / `cosmic-text` / `swash`。
- **周边 UI(面板/设置)**:`egui` + `egui-wgpu`,与终端同帧渲染(终端经 `PaintCallback`)。
- **仅作参考**(不作为地基):Rio 的 `rio-vt`/`sugarloaf`、WezTerm 的 `termwiz`、`libghostty`
  (迭代频繁 / 不稳定 / 无 Windows)。

## 更新

引导阶段屏幕模型用 `vt100`;Phase 2 起 app 运行在 `alacritty_terminal` 上(经 `term-core::aterm`),`vt100` 已移除。

Update:ADR 0030 增加了第二个宿主 `mtty-widget`(bin `miaotty-native`),其原生
`winit` + `wgpu` 循环直接绘制网格;终端不再总是经 egui `PaintCallback`,该路径现仅
服务于 `eframe` 宿主。

## 后果

- 快速到"可用且快"的终端;全程 Apache/MIT(可被嵌入)。
- 渲染与应用层自持;不复用 Ghostty 的渲染/shell/配置生态(提供配置导入做迁移)。
- 风险集中在自绘渲染器与 Windows IME,由 R0/R0.5 spike 门控。
