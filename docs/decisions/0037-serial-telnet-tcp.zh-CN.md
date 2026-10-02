# ADR 0037 — 串口、Telnet 与裸 TCP 会话

> English: [`0037-serial-telnet-tcp.md`](0037-serial-telnet-tcp.md)

状态:已接受。

## 背景

M6(PuTTY 风格的远程)从 R1 开始:串口控制台(波特率、数据位、校验、停止位、流控)、
Telnet 与裸 TCP,与 SSH 一起保存在主机库中。目前每个 pane 都是 PTY 上的 shell:
`miao-term-core` 的 `Terminal` 启动 `MasterPty` 与子进程,并在后台线程读取 master
(`crates/term-core/src/term.rs`);主机库(`miao-term-config::hosts`)只保存 SSH 目标。

串口控制台、Telnet 服务器与裸 TCP 对端都没有 shell、没有 PTY,也没有 OSC 133 的命令
边界——它们只是一个进出的字节流。因此终端核心必须能直接消费和写入一条普通传输,而不能
假设背后有进程。

## 决定

1. **终端核心与传输解耦。** 新增
   `Terminal::from_pipe(cols, rows, scrollback, reader, writer, waker)`;
   核心只假设一个 `Read`,由后台线程把字节送入 `rx`,以及一个
   `Box<dyn Write + Send>`。`Terminal::new` 保留 PTY 路径,只是从 master 构造同一条管道。
   `master` 与 `child` 变为可选,因此没有进程的会话在销毁时不会去结束任何进程。仅 PTY 需要
   的钩子(shell 集成、来自 OSC 7 的工作目录、命令捕获)在没有 shell 时保持不生效。
2. **传输后端**放在 `miao-term-ui`(不依赖 GPU,便于单元测试),每个后端一个模块:
   - **裸 TCP** —— `std::net::TcpStream`,无协议。
   - **Telnet** —— `TcpStream` 之上的一个内置小型编解码器:应答 IAC 协商
     (WILL/WONT/DO/DONT,除字符集与已有回显外一律拒绝),从核心看到的字节流中剥离
     IAC 序列,写入时对 IAC 做转义。选择自己实现而非引入 crate:需要的选项集很小,
     可把依赖与许可证面保持为零(ADR 0006)。
   - **串口** —— `serialport` crate(MIT),按保存的配置(波特率、数据位、校验、
     停止位、软件与硬件流控)打开。
3. **主机库。** `hosts.toml` 的 `Host` 增加 `kind`(默认 `ssh`,另有 `serial`、
   `telnet`、`tcp`)及对应的字段(地址与端口,或串口配置)。从 `~/.ssh/config` 导入
   只创建 `ssh` 条目;此前的文件照旧读取(缺少 `kind` 视为 `ssh`)。
4. **界面。** 主机列表、Open Quickly、`mtty://host/<name>` 与命令面板都提供新的类型;
   串口会话的对话框(设备、波特率、校验、数据位、停止位、流控)是一个按配置预填的小表单。
   Telnet 与裸 TCP 不加密,会话在侧栏中按明文标注,与明文 FTP 一致,mtty 不为其保存任何
   凭证。设备打不开时,在 pane 中显示操作系统返回的错误。
5. **生命周期。** 传输关闭或出错时,pane 像 shell `exit` 一样结束;重连会按同一份保存的
   配置重新打开,会话恢复即重连,与 SSH 会话相同。非 PTY 传输不宣传 shell shim、
   OSC 7 工作目录或命令捕获。

## 影响

- 新增一个依赖 `serialport`(MIT);Telnet 与 TCP 不新增依赖。
- 终端核心不再假设子进程存在,这也让原生 SSH 方案(R3)成本更低。
- 非 PTY pane 没有工作目录(侧栏显示传输信息),也没有命令边界;需要 shell 的
  recipe 或 agent 会明确拒绝它。
- "复制/发送上一条命令的输出"与 agent 队列不适用于这些会话,界面会说明而不是静默失败。
- Windows 串口经 `serialport` 可用;配置表单会列出系统的 COM 端口与 Unix 设备节点。
