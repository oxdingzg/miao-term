# ADR 0024 — MTP view/edit 与 file read/write

> English (default): [`0024-mtp-view-edit.md`](0024-mtp-view-edit.md)

状态:已接受。

## 背景

ADR 0021 用 ssh 提供了远端 view/edit,但**被隧道的**客户端(远端 shell 把 `MIAOTTY_SOCKET`
指向应用 socket)仍无法请应用打开文件、也无法读/写文件,因为这些动词在控制面不存在。
此前 `term-mtp` 之所以被卡住,只是因为工作树有未提交改动 —— 而查明那其实是本会话 `cargo fmt`
的产物,并非他人的工作,故可以安全扩展。

## 决定

扩展 MTP 面(`crates/term-mtp`):

- **`app.view` / `app.edit`** —— `{ path }`;入队 `Command::View/Edit(path)`。应用把文件以只读
  方式在查看器(`view`)或编辑器(`edit`)中打开,故客户端可以驱动 UI。
- **`file.read`** —— `{ path }` → `{ data, bytes, truncated }`,上限 `MAX_FILE_BYTES`(2 MB)
  并带 `truncated` 标志;IO 错误变为 `io_error` 响应。
- **`file.write`** —— `{ path, data }` → `{ ok, bytes }`。
- `core.ping` 中广告能力:`app.view.write`、`file.read`、`file.write`。

CLI 增加对应子命令:`miaotty-cli view <path>`、`miaotty-cli edit <path>`、
`miaotty-cli file read|write --path P [--data D]`。

## 后果

- 隧道或本地客户端现在可在运行中的应用里打开文件,并经 host 读写文件,补上了 M4 的
  "远端 pane 可 view/edit"。
- `file.read` 有上界并返回 `truncated`,客户端无法让 host 读无界文件;`file.write` 是 UTF-8
  文本,足以支撑查看/编辑流程。
- 鉴权仍是"能连到 socket 即可"(socket 为用户私有,0600);更细的能力门控与二进制/流式传输
  属后续。
