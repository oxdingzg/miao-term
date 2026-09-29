# ADR 0005 — MTP 传输

> English (default): [`0005-transport.md`](0005-transport.md)

状态:已接受。

## 背景

控制面(MTP)要能被 `miaotty-cli`、插件、agent hooks 在三平台上访问,且不与引擎耦合。

## 决定

- **报文**:换行分隔 JSON(NDJSON),沿用现有 `miaotty` 的 MTP 信封
  (`v/id/kind/ns/method/params` → `v/id/kind/ok/result/error/revision`)。
- **传输**:用 `interprocess`(同一 API → macOS/Linux 的 Unix socket `$TMPDIR/miaotty.sock`,
  Windows 命名管道 `\\.\pipe\miaotty`)。路径经 `MIAOTTY_SOCKET` 导出;Unix 下 socket 文件权限 `0600`。
- **进程内 UI** 直接读注册表(不走 socket);外部调用走 socket/pipe。
- 请求有上界(`MAX_LINE`),单客户端无法撑爆内存。

## 后果

- 现有 `miaotty-cli` 无需改动即可连 Rust host(已验证)。
- Windows 需要在同一接口后加一个小传输层。
- socket server 崩溃不会拖垮 UI(独立线程)。
- Update:socket 路径现优先 `$XDG_RUNTIME_DIR/miaotty.sock`,回退 `$TMPDIR/miaotty.sock`,
  以便在平台提供用户私有目录时避开共享位置。
