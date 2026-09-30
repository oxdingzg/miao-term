# ADR 0031 — 把应用菜单放进 macOS 菜单栏

> English (default): [`0031-native-app-menu.md`](0031-native-app-menu.md)

状态:已接受。

## 背景

外壳原先在窗口内自绘一条菜单(文件 / 编辑 / 视图 …),`miaotty-app` 与 `miaotty-native`
都如此。但在 macOS 上这看起来很"外来":别的终端(包括 Otty)都把菜单放在系统菜单栏,窗口本身
没有菜单条。`winit` 0.30 没有菜单 API,因此需要平台库:`muda`(MIT,tauri 的菜单库)。

但 `muda` 有两处危险,都在这里复现过:

- 它 macOS 的图标路径在把图标转成 `NSImage` 时 `unwrap`。没有应用图标的进程会产出零尺寸图标,
  `png` 拒绝它,而 panic 发生在无法 unwind 的 AppKit 菜单回调里 —— 点击系统标准项 *About*
  直接以 `FormatError { inner: ZeroWidth }` 中止进程。
- 原生 `NSMenuItem` 持有指向 Rust 侧 `MenuChild` 的裸指针。把 `MenuItem` 追加后丢掉(最直觉的
  循环写法)会让该指针悬垂,点击我们自己的菜单项时以
  `CFString cannot be created from a negative number of bytes`(SIGTRAP)中止。

## 决定

macOS 上把菜单放进系统菜单栏,并保持诚实:

1. 用一张表 `miao_term_ui::menu::menus(lang)` 描述菜单。原生宿主据此构建系统菜单栏;
   `chrome::render` 在宿主允许时(`Chrome::draws_menu_bar`)用同一张表绘制窗口内菜单。
2. 系统菜单栏**仅在 macOS 且二进制位于 `.app` bundle 内**时安装(`menu_in_os()`)。图标就在
   bundle 里,而 AppKit 的 About 面板需要它;同时让裸跑的 `target/release/miaotty-native`
   继续使用窗口内菜单,开发时永远不会因为点菜单而中止。bundle 由
   `scripts/package-macos.sh` 产出(`miaotty-native.app`)。
3. `muda` **vendor** 到 `vendor/muda`,带一份最小且写明的补丁
   (`vendor/muda/PATCHES.miao.md`):零尺寸图标改为编码为 1×1 透明图,无法解码的图回退为空
   `NSImage`,而不是在回调里 panic。
4. 宿主创建的每个 `MenuItem`/`Submenu` 都在 `Host` 里**保活**
   (`appmenu::MenuHandle`),因为 muda 的原生项指向它们。

窗口同时向系统请求深色外观(`with_theme(Dark)`):界面本身是深色,旁边一条浅色标题栏看起来像
外来物——Otty 也是把整个外框按主题着色。

## 后果

- macOS 得到原生菜单栏(含 About / Services / Hide / Quit),窗口内不再有菜单条,与 Otty 一致;
  Linux 与 Windows 仍用窗口内菜单,eframe 宿主不变。
- 菜单项带真实加速键,故安装菜单期间这些快捷键由 AppKit 接管(裸跑时仍由我们的按键处理兜底)。
- 仓库里多约 380 KB 第三方源码,升级 `muda` 时需重新套用一个小补丁(见补丁文件)。这是"不发布
  一个点菜单就崩的版本"的代价,予以接受。
- 已在 bundle 版上手工验证:About、Hide Others、Show All、文件→新建标签、终端→向右分屏、
  编辑→全选、视图→开关侧栏、退出均正常,进程不崩。
