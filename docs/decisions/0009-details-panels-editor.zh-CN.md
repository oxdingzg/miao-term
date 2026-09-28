# ADR 0009 — Details 面板与文件编辑器

> English (default): [`0009-details-panels-editor.md`](0009-details-panels-editor.md)

状态:已接受。

## 背景

右侧 Details 栏不能只显示工作目录。roadmap 的 U5 要求 git、files、ports 三个面板,
以及一个双击打开的文件查看器。这些都读取*焦点 pane* 的上下文(cwd 与子进程 pid),
且不得阻塞 UI 线程。

## 决定

**面板。** Details 改为标签页:Info、Agent、Outline、Git、Files、Ports。
Info/Agent/Outline 读现有状态;Git/Files/Ports 读后台 worker 产出的快照。

**后台 worker**(`miaotty-app/src/panels.rs`)。一个线程持有请求通道与共享的
`Arc<Mutex<Panels>>` 槽。UI 在焦点 pane 上下文变化时、或每 2.5 秒,发送
`Request { cwd, pid }`;worker 合并为最新请求,计算并发布。UI 只读克隆,故永不阻塞。
- Git:`git -C <cwd> rev-parse --abbrev-ref HEAD` 与 `status --porcelain`。
- Files:`read_dir`,目录在前,其次按名称大小写不敏感排序。
- Ports:`lsof -nP -iTCP -sTCP:LISTEN -a -p <pid>`(无 `lsof` 的平台如 Windows 为空)。

**编辑器 / Quick Look。** 在 Files 或 Git 面板中双击文件,会在窗口中打开,且**默认只读**
(Preview),带 Edit 切换、Reload、Save。超过 2 MB 的文件拒绝打开;非 UTF-8 字节按 lossy
解码,故查看器不会因二进制或超大输入崩溃。Markdown/Mermaid 渲染推迟。

**面板(命令面板)。** Open Quickly 增加焦点目录下的 `file` 条目,其 `OpenFile` 动作路由到
同一个编辑器。

## 后果

- 即便 `git status` 或 `lsof` 很慢,面板仍保持响应;UI 在下一次快照到达前显示上一份。
- worker 是普通线程,而非 async 运行时 —— 与引擎的线程模型一致(ADR 0002)。
- 符号搜索、`jump` frecency 与富预览(Markdown/Mermaid)属后续;本 ADR 固定它们所依赖的管道。
