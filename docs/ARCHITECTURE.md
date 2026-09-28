# miao-term 架构规格

> 跨平台(macOS / Linux / Windows)终端**引擎** + `miaotty` 应用的整体架构。
> 本文是**实现前的定稿设计**;所有"锁定"条目即为决策,变更须走 `docs/decisions/` 的 ADR。

## 0. 范围与一句话

一句话:**`portable-pty` + `alacritty_terminal` + `vte` 做内核,`winit` + `wgpu` 做窗口与绘制,终端网格自绘、周边 UI 用 egui;引擎(`miao-term-*`)与应用(`miaotty-app`)分离,控制面(MTP)与引擎解耦。**

## 1. 目标 · 非目标 · 约束

**目标**
- 三平台同一套代码,含 **Windows(ConPTY)**。
- 热路径性能对齐 Alacritty 量级:输入延迟 P95 ≤ 16ms(目标 ≤ 8ms)、首帧 ≤ 100ms、滚动不丢帧、空闲 CPU ≈ 0。
- 引擎可被第三方嵌入(`examples/` 证明);`miaotty-app` 是第一个消费者。
- 复用现有控制面:`mtp` 类型、`miaotty-cli`、插件、agent/shell hooks。

**非目标(现阶段)**
- 复刻 Ghostty 的渲染精细度与配置生态;macOS 专属集成(AppleScript/Sparkle)。
- 插件市场/"框架"级扩展;SSH/远程访问;跨机同步。

**约束**
- 依赖只用宽松许可(Apache-2.0/MIT/BSD/ISC);**引擎内禁止 GPL/AGPL**(如 Pebrel 只能读、不能抄)。
- 每增加一个外部 crate 都要过性能与许可审查。
- Rust stable,MSRV 见 `rust-toolchain.toml`。

## 2. 锁定决策(摘要)

| # | 决策 | 理由 |
|---|------|------|
| D1 | 内核用 `alacritty_terminal` + `vte` + `portable-pty` | 最成熟、跨平台(含 ConPTY)、Otty 同款、许可宽松 |
| D2 | 渲染自绘于 `wgpu`(字形用 `glyphon`/`cosmic-text`/`swash`) | 性能可控;跨 Metal/Vulkan/DX12 |
| D3 | 周边 UI(面板/设置)用 `egui`,与终端**共用同一帧** | 开发速度 + 终端走自绘,两全 |
| D4 | **自研 tab/split 模型**(非 OS 原生标签) | 跨平台一致;对齐 Otty;可控 |
| D5 | 并发用 **Alacritty 同款的锁纪律**(`FairMutex<Term>` + `EventListener`) | 已验证,避免自造快照协议 |
| D6 | 平台差异只出现在 `core::pty` / `widget::platform` / `mtp::transport` | 收敛复杂度 |
| D7 | 控制面 `term-mtp` 与引擎解耦(Unix socket / Windows named pipe) | 引擎崩不拖垮 CLI;复用现有协议 |
| D8 | 先做 app、后抽库;扩展点分阶段 | 由真实需求驱动 API |
| D9 | 引擎 crate 双许可 `MIT OR Apache-2.0` | 便于被嵌 |

## 3. 依赖分层(DAG)与规则

```
            term-config      term-mtp          (独立,无引擎依赖)
                 │               │
   term-core ──► term-render ──► term-widget
        ▲                                ▲
        └──────────── miaotty-app ───────┘   (依赖全部)
```

**规则(CI 强制)**
- 依赖只能"由外向内":`widget → render → core`;`core` 不反向依赖任何引擎内其它 crate。
- `core` 不依赖 `wgpu`/`winit`/`egui`(**可无 GPU 编译**);`render` 不依赖 `winit`。
- `config`/`mtp` 不依赖渲染与窗口。
- 应用层依赖引擎;引擎**绝不**依赖应用层。
- 用 `cargo-deny` 查许可/漏洞,`cargo-machete` 查未用依赖。

## 4. crate / 模块职责矩阵

| crate | 职责(做什么) | 明确不做 |
|-------|--------------|----------|
| `term-core` | PTY、vte 解析、网格/回滚/光标/模式、选区/查找、OSC/CSI 语义、键鼠→字节编码、事件(`EventSink`) | 不碰 GPU/窗口/配置/业务 |
| `term-render` | 字形加载/shaping/图集、网格实例化、绘制 pass、damage 增量 | 不管事件循环/输入 |
| `term-widget` | winit 事件循环、wgpu surface、输入/IME/剪贴板/拖放、egui 组合、`Host` 回调 | 不含 tab/面板业务 |
| `term-config` | 配置模型、主题、ghostty/alacritty 导入 | 不依赖 UI |
| `term-mtp` | 协议信封、传输、server/client、agent/history 注册表、事件订阅 | 不依赖引擎 |
| `miaotty-app` | 窗口/tab/split、左 Tabs、右 Details、徽章、设置、系统集成、hook 安装 | 不重复实现终端内核 |

## 5. 核心类型与 trait(Rust 草图)

```rust
// ---- term-core ----
pub struct Term { /* grid + scrollback + cursor + modes */ }
pub struct Damage { pub full: bool, pub lines: RangeSet<usize> }

pub enum TermEvent {
    Title(String), Cwd(PathBuf), Bell, Progress(Progress),
    ClipboardStore(String), ClipboardLoad(u8),
    PromptStart, CommandStart, CommandDone(i32),     // OSC 133
    PtyWrite(Vec<u8>), PtyExit(i32, Option<i32>),    // 需由宿主写回 PTY / 关闭
    Wakeup,                                          // 有新输出 → 需要重绘
}
pub trait EventSink: Send + 'static { fn send(&self, e: TermEvent); }

pub trait Pty: Send {
    fn write(&self, bytes: &[u8]) -> io::Result<()>;
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()>;
    fn reader(&self) -> io::Result<Box<dyn Read + Send>>;  // 读线程持有
}

pub trait InputEncoder { fn encode(&self, ev: &InputEvent) -> Vec<u8>; }

// ---- term-render ----
pub struct GlyphAtlas { /* R8 texture + LRU */ }
pub trait Renderer {
    fn update(&mut self, term: &Term, damage: Damage);          // 组实例(短锁)
    fn render(&mut self, frame: &mut wgpu::RenderPass);         // 提交 GPU
}

// ---- term-widget ----
pub trait Host: Send + Sync {
    fn set_title(&self, s: &str);
    fn open_url(&self, url: &str);
    fn notify(&self, title: &str, body: &str);
    fn clipboard(&self, kind: Clipboard, data: Option<String>) -> Option<String>;
    fn request_redraw(&self);
}
```

## 6. 线程模型与锁纪律

采用 Alacritty 验证过的模型(`FairMutex<Term>` + `EventListener`),不自定义快照协议。

```
┌─ PTY 读线程 ───────────────────────────┐     ┌─ 主线程(winit 事件循环) ─────────────┐
│ loop read():                           │     │ winit event:                          │
│   lock(term) { vte.process(chunk) }    │     │   Keyboard/IME → core::input → pty    │
│   标 Damage; sink.send(Wakeup)         │     │   RedrawRequested:                    │
│   （锁仅覆盖解析,不含 I/O 与 GPU）      │     │     lock(term){ renderer.update }     │
└────────────────────────────────────────┘     │     renderer.render(frame); present   │
                                               └───────────────────────────────────────┘
```

**不变量**
1. `Term` 的锁**只**覆盖"解析一段字节"或"构建渲染实例",**绝不**跨越 `read()`/`write()`/GPU submit。
2. PTY 读线程**独占** reader;写走独立句柄(`Pty::write`),避免读写互相阻塞(Windows ConPTY 尤其)。
3. 空闲时无轮询;只有 `Wakeup`/输入/定时器(光标眨眼)才请求重绘。
4. resize:主线程算 `cols/rows` → `Pty::resize`;`Term` 同步 resize。
5. 退出码以 **OSC 133;D** 为准(ConPTY/包装进程下进程码不可信)。

## 7. 数据流

- **输出**:PTY → 读线程 → `vte` → `Term`(+OSC→`TermEvent`)→ `Wakeup` → 主线程渲染。
- **输入**:winit 键/鼠/IME → `core::input` 编码 → `Pty::write`;选区/粘贴走 bracketed paste 模式。
- **控制**:`term-mtp` server 独立线程;进程内 UI 直连注册表,外部 CLI/插件走 socket/pipe。
- **元数据**:cwd(OSC 7)、标题、agent 状态、命令历史 → 事件/注册表 → 面板订阅。

## 8. 渲染架构(term-render)

1. **字形**:`cosmic-text` 解析/回退,`swash` 光栅化 → **R8 图集**(shelf packing,LRU;键 `(glyph_id, style, px, subpixel)`).
2. **实例化**:每个 Cell → quads(bg / glyph / underline / strike / cursor / selection);实例缓冲按脏行增量更新。
3. **Pass**:`bg`(纯色/主题)→ `glyph`(图集采样)→ `cursor/decoration`;透明度/背景图后置。
4. **Present**:优先 `Mailbox`(低延迟),掉帧回退 `Fifo`;光标杆眨眼用定时器。
5. **与 egui 组合**:egui 驱动整帧,终端通过 `egui-wgpu` 的 **`PaintCallback`** 在自己的矩形内绘制(独立 pipeline,复用同一 `wgpu::Device/Queue/Surface`)。
6. **优化顺序**:全量重建 → 脏行 → 图集命中 → 去 per-frame 分配;用 `tracy`/`puffin` 标注热点。

## 9. 终端模型(term-core)

- 网格:Cell{char + 组合 + fg/bg/attrs + underline 样式};回滚环形缓冲;reflow(宽窗口)。
- 模式:应用光标键、bracketed paste、鼠标上报、alternate screen、kitty keyboard(CSI u)、焦点上报。
- 语义:标题、超链接(OSC 8)、cwd(OSC 7)、进度(OSC 9;4)、shell 集成标记(OSC 133 A/B/C/D)。
- 选区/查找:行/块选区、词边界(CJK/grapheme 用 `unicode-width` + grapheme 边界)、搜索高亮。
- 图形协议(kitty graphics / sixel / iTerm2)列为 R1+ 增量,不在 R0。

## 10. 平台抽象层

| 关注点 | 抽象 | macOS/Linux | Windows |
|--------|------|-------------|---------|
| PTY | `trait Pty` | `forkpty` | **ConPTY** |
| 传输 | `mtp::transport` | Unix socket | `\\.\pipe\miaotty` |
| 剪贴板 | `trait Clipboard` | NSPasteboard / X11-Wayland | Win32 clipboard |
| 字体 | `term-render::font` | CoreText / fontconfig | DirectWrite(`font-kit`) |
| IME | `widget::input` | 原生 | **TSF**(风险最高,见 §19) |

`#[cfg(...)]` **只允许**出现在上表对应模块内;其余代码保持平台无关。

## 11. 配置 / 主题

- `term-config`:自有 TOML;键名对齐 ghostty/alacritty 以便导入。
- 默认观感对齐 Otty(Nord 背景 `#2e3440`、字号 14),可覆盖。
- 主题:内置常用主题 + 自定义调色板;后续支持背景图/透明度。

## 12. 控制面(MTP)

- `term-mtp` 实现 server;传输:Unix socket(`$TMPDIR/miaotty.sock`)/ Windows named pipe。
- 复用现有 `mtp` 报文与 `miaotty-cli`;**进程内 UI 直连注册表**,外部走 socket/pipe。
- 方法面:`core.ping/health`、`agent.state.*`、`history.*`、`pane.list`;事件:`agent.state`、`history.changed`、`cwd.changed`。
- 传输实现候选 `interprocess`(待评估许可/维护),否则自写薄封装。

## 13. 应用层(miaotty-app)

- 模型:`Window → Tab[] → SplitTree<Surface>`;surface = 一个终端实例(core+render 视图)。
- 窗口:一个 OS 窗口承载一个 tab 集;分屏是 tab 内的树(自研,非 OS 标签)。
- 面板(egui):**左 Tabs 侧栏**(标题/⌘N/前缀/标记/分隔线/右键菜单)、**右 Details**(Info/Outline/Git/Files)、徽章、设置、命令面板。
- 视觉对齐 Otty:面板与终端同背景、无分隔线、hover 高亮。
- 系统集成:hook 安装、URL 打开、通知、防休眠(平台分支)。

## 14. 扩展点(分阶段,别提前)

- **现在**:`Host` trait + `EventSink`(够用)。
- **R4+**:MTP `provider.*`(Details 自定义组件,kind=tui/web)。
- **以后(真有人要)**:渲染器/面板插件、主题包 —— 才考虑"框架"化。

## 15. 错误处理与安全

- 库用 `thiserror`、应用用 `anyhow`;**热路径不 panic**;可恢复错误转为事件/日志。
- 安全:粘贴确认、OSC 注入面、URL scheme 白名单、剪贴板策略、可选 secure input。
- 崩溃:捕获并写日志(panic hook),尽量保 PTY/子进程清理。

## 16. 测试 / CI / 性能门

- **一致性**:`vttest`/`esctest` 子集 + golden grid 断言;解析器 fuzz(`cargo-fuzz`)。
- **集成**:真起 shell,喂字节序列,断言网格/事件。
- **性能门(三平台,Windows 单列)**:输入延迟 P95 ≤ 16ms、首帧 ≤ 100ms、`cat` 大文件不丢帧、空闲 CPU ≈ 0(用 `criterion` + 自建延迟 harness,结果入库防回归)。
- **CI**:mac/linux/windows 三矩阵 `check`/`test`/`deny`/`fmt`/`clippy`。

## 17. 打包 / 发布 / 版本

- 引擎 crate `MIT OR Apache-2.0`,先内部用,API 稳定后发布 crates.io。
- 应用:macOS `.app`+notarize;Linux(AppImage/Flatpak/.deb);Windows MSI(`cargo-dist`/`cargo-wix`)+ 代码签名(Azure Artifact Signing 或自签)。
- `wgpu` DX12 需随包 `dxcompiler.dll` 或静态 `static-dxc`。

## 18. 里程碑

R0 最小闭环(pty→vt→grid→render→input,量延迟) → **R0.5 IME 专项** → R1 可用终端 → R2 三平台+打包 → R3 `term-mtp` → R4 面板 → R5 打磨/签名。每阶段退出须过对应性能门。

## 19. 风险

| 风险 | 缓解 |
|------|------|
| 自绘渲染达不到 Alacritty 级 | glyphon 起步→damage/vsync/图集优化;参考 Alacritty/Rio |
| **IME/CJK(Windows 最险)** | R0.5 专项;winit `Ime` + `set_ime_cursor_area`;字宽/grapheme 测试集 |
| ConPTY 性能/退出码 | 读写并发 + OSC 133;D + 独立基准 |
| egui 与自绘共帧的兼容/性能 | `PaintCallback` 方式;必要时退化为手动 viewport |
| 早期过度抽象 | 引擎先"够用";扩展点分阶段 |
| 上游 `alacritty_terminal` API 变动 | pin 版本;封装 `term-core` 适配层 |

## 20. 待补 ADR(`docs/decisions/`)

- 0001 技术栈选型(本文 §2 固化)
- 0002 并发与锁纪律
- 0003 渲染/egui 共帧方案
- 0004 自研 tab/split(非 OS 原生)
- 0005 MTP 传输(Unix socket / named pipe)
- 0006 许可与依赖策略
