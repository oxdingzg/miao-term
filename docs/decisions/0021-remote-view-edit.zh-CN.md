# ADR 0021 — 经 ssh 的远端 view/edit

> English (default): [`0021-remote-view-edit.md`](0021-remote-view-edit.md)

状态:已接受。

## 背景

M4 的验收是"远端 pane 可 view/edit"。miaotty 惯常的做法是由 host 应答某个 MTP 方法,
但 `term-mtp` crate 目前带着另一位作者的未提交改动,改它的 dispatcher 会把两人的工作混在
一起。

## 决定

**经由既有的 ControlMaster ssh 连接**实现该能力,不改 MTP、也无需在远端安装任何东西:

- `ssh.rs` 新增 `read_remote(dest, path)`(`ssh … dest 'cat -- <path>'`)与
  `write_remote(dest, path, data)`(`ssh … dest 'cat > <path>'`,数据走 stdin),两者都复用
  `ControlMaster=auto` / `ControlPersist=60s` 与 `BatchMode=yes`,单次读取上限 2 MB。argv
  构造是纯函数并有单测(`read_args`、`write_args`、引用转义)。
- Open Quickly 新增 *View Remote File…* 与 *Edit Remote File…*。它们弹出一个两字段对话框
  (ssh 目标、远端路径);读取在线程里进行,UI 永不阻塞,结果在查看器(view)或编辑器(edit)
  中打开。
- 编辑器记住 `RemoteRef`;此后 Save 经 ssh 写回、Reload 重新读取,故对用户而言远端文件与
  本地无异。

## 后果

- 远端 view/edit 今天就可用,复用 pane 本已使用的同一条连接,零远端安装 —— 与 M4 的
  零安装兜底一致。
- 读写路径的可用性取决于 ssh(ControlMaster 已建立、`BatchMode` 所需的密钥/agent 认证);
  失败会显示在状态行,而不是卡住。
- 把 `app.view` / `app.edit`(及 `file.read` / `file.write`)暴露为一等 MTP 方法,以便
  *被隧道*的客户端无需本地运行我们的二进制即可触发,仍属后续 —— 前提是先落地
  `term-mtp` 的待提交改动。
