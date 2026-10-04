# ADR 0024 — MTP view/edit 与 file read/write

> English (default): [`0024-mtp-view-edit.md`](0024-mtp-view-edit.md)

状态:已接受。

## 背景

ADR 0021 用 ssh 提供了远端 view/edit,但**被隧道的**客户端(远端 shell 把 `MIAOTTY_SOCKET`
指向应用 socket)仍无法请应用打开文件、也无法读/写文件,因为这些动词在控制面不存在。
此前 `mtty-mtp` 之所以被卡住,只是因为工作树有未提交改动 —— 而查明那其实是本会话 `cargo fmt`
的产物,并非他人的工作,故可以安全扩展。

## 决定

扩展 MTP 面(`crates/mtty-mtp`):

- **`app.view` / `app.edit`** —— `{ path }`;入队 `Command::View/Edit(path)`。应用把文件以只读
  方式在查看器(`view`)或编辑器(`edit`)中打开,故客户端可以驱动 UI。
- **`file.read`** —— `{ path }` → `{ data, bytes, truncated }`,上限 `MAX_FILE_BYTES`(2 MB)
  并带 `truncated` 标志;IO 错误变为 `io_error` 响应。
- **`file.write`** —— `{ path, data }` → `{ ok, bytes }`。
- `core.ping` 中广告能力:`app.view.write`、`file.read`、`file.write`。

CLI 增加对应子命令:`miaotty-cli view <path>`、`miaotty-cli edit <path>`、
`miaotty-cli file read|write --path P [--data D]`。

## 附记(鉴权、二进制传输、被隧道的客户端)

- **令牌**:host 以 `MIAOTTY_MTP_TOKEN` 启动时,每个请求都必须携带匹配的 `token` 字段,否则返回
  `unauthorized`。客户端读取同一环境变量并自行注入该字段。
- **offset/length + base64**:`file.read` 接受 `offset`、`length`(上限 `MAX_FILE_BYTES`)与
  `encoding = "base64"`,并回报 `bytes/offset/returned/eof/truncated`;`file.write` 接受
  `data_b64`。因此二进制文件可按有界分片传输。base64 在 crate 内实现(无新依赖)。
- **被隧道的客户端**:已端到端验证(Mac 为 host、Linux 为客户端):用
  `ssh -R /tmp/fwd.sock:<host socket>` 转发 socket,并在远端运行
  `miaotty-cli --socket /tmp/fwd.sock ping` —— 不带令牌被拒,带令牌 `ping` 返回能力列表,
  且 base64 写入+读取往返得到相同字节。

## 后果

- 隧道或本地客户端现在可在运行中的应用里打开文件,并经 host 读写文件,补上了 M4 的
  "远端 pane 可 view/edit"。
- `file.read` 有上界并返回 `truncated`,客户端无法让 host 读无界文件;`file.write` 是 UTF-8
  文本,足以支撑查看/编辑流程。
- 鉴权仍是"能连到 socket 即可"(socket 为用户私有,0600);更细的能力门控与二进制/流式传输
  属后续。
- Update:随后加入了 `remote-listen` TCP 监听(要求 `MIAOTTY_MTP_TOKEN`)与 `MIAOTTY_MTP_ALLOW`
  能力白名单,故鉴权不再只是"能连到 socket 即可";本地 socket 仍为用户私有(0600)。

- Update:协议另外加入了 `core.wait`(对状态 revision 的长轮询)与 `core.subscribe`——后者把
  连接升级为推送式事件流,覆盖 `agent.state`、`panes`、`history` 三个 topic
  (`miaotty-cli events [--topic T]`);客户端关闭连接即退订。
