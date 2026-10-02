# ADR 0038 — 导入 PuTTY `.ppk` 私钥

> English: [`0038-ppk-import.md`](0038-ppk-import.md)

状态:已接受。

## 背景

M6 R2。Windows 用户常把 SSH 密钥放在 PuTTY 的 PPK 格式里,而 mtty 所有连接都走
OpenSSH,读不了该格式。现实中存在两个版本:**v2**(PuTTY 0.52–0.74)用 SHA-1 从口令
派生 AES 密钥与 IV,并用 HMAC-SHA-1 保护文件;**v3**(0.75+)改用 Argon2id 与
HMAC-SHA-256。密钥通常是 RSA 或 Ed25519,也有 ECDSA。

需求是单向导入:读取 `.ppk`,校验,写出一个 OpenSSH 私钥。写出的 OpenSSH 私钥
**绝不能以未加密形式落盘**。

## 决定

1. **新增不依赖 GPU 的 `miao-term-keys` crate。** 它解析 PPK v2 与 v3,在使用任何
   密钥材料之前先校验 `Private-MAC`,解密私钥数据,并以 `ssh-key` 的 `PrivateKey`
   返回。它有单元测试,不含窗口代码,便于单独审计与模糊测试。
2. **支持的密钥类型**:Ed25519、RSA 与 ECDSA(NIST P-256/P-384/P-521)。DSA 直接
   拒绝并说明原因(已过时;OpenSSH 9.8 起禁用)。
3. **支持的加密**:v2 的 `none` 与 `aes256-cbc`;v3 的 `none`、`aes256-cbc` 与
   `chacha20-poly1305`(Argon2id,与 PuTTY 写出的格式一致)。MAC 始终校验,口令错误
   会在密钥暴露前的一步失败。Argon2 参数取自文件中的 `Argon2-*` 头。
4. **输出始终加密。** 导入的密钥经 `ssh-key` 的 OpenSSH 写出器,以 bcrypt-pbkdf +
   `aes256-ctr` 和用户在导入时设置的口令重新加密。新口令为空即拒绝;不存在“保存为
   未加密”的路径。
5. **界面。** *主机…* 增加 **导入 PuTTY 密钥…**:选择 `.ppk`,若加密则输入其口令,
   再输入新口令(两次)。密钥写到 `~/.ssh/<name>`(0600)与旁边的 `<name>.pub`
   (0644);已存在时拒绝覆盖,除非用户确认。口令与解码后的密钥保存在会 `zeroize`
   的缓冲区中,绝不记日志或出现在命令行上。
6. **依赖**(均为 MIT 或 Apache-2.0,遵循 ADR 0006):`aes`、`cbc`、`sha1`、
   `sha2`、`hmac`、`argon2`、`chacha20poly1305`、`zeroize`,以及启用
   `encryption`、`alloc`、`ed25519`、`rsa`、`p256`、`p384`、`p521` 特性的
   `ssh-key`。OpenSSH 编码器同样由 `ssh-key` 提供。

## 影响

- 导入是纯 Rust、进程内完成:任何平台都不需要 `puttygen`,在没有 OpenSSH 自带工具的
  Windows 上也可用。
- 不支持 PPK v1(0.52 之前的短暂格式);这类文件会明确报错,而不会被误解析。
- 转换刻意保持单向。不计划把 OpenSSH 密钥导出回 PPK;PuTTY 本身能读 OpenSSH 密钥。
- 今后任何改动都必须保持“绝不明文落盘”的规则;测试断言未加密输出不可能出现
  (空口令被拒绝)。
