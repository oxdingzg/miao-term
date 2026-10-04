# ADR 0006 — 许可与依赖策略

> English (default): [`0006-license-policy.md`](0006-license-policy.md)

状态:已接受。

## 背景

引擎(`mtty-*`)要能被第三方嵌入,应用(`miaotty-app`)是它的第一个消费者。
我们依赖的 Rust 生态绝大多数是宽松许可,但有少数间接依赖带"二选一"的 copyleft
*选项*(例如 `Apache-2.0 OR GPL-2.0-only`)。因此需要明确一个对外许可,以及依赖准入规则。

## 决定

- **对外**:引擎各 crate 与 `miaotty-app` 采用 **Apache-2.0**(见根目录 [`LICENSE`](../../LICENSE))。
  Apache-2.0 宽松、可嵌入,并含明确的专利授权与商标条款。
- **对内(依赖)**:只允许宽松许可。白名单:`Apache-2.0`、`MIT`、`BSD-2-Clause`、
  `BSD-3-Clause`、`ISC`、`Zlib`、`0BSD`、`CC0-1.0`、`Unicode-3.0`、`OFL-1.1`、
  `BSL-1.0`,以及任何"提供上述之一作为可选项"的表达式。
- **引擎内禁止 copyleft**:GPL/AGPL/SSPL 等一律排除;带 copyleft *可选*项
  的 crate,只有在选择宽松选项时才可用。
- **审查**:以 `cargo-deny`(许可 + 安全公告)作为 CI 门控;当前在评审阶段执行,
  未通过不得合入新依赖。

## 后果

- 单一许可,消费者不再面对"双许可、任选其一"的歧义。
- 若某依赖不再提供宽松选项,须替换或锁死版本。
- 若打包了附带 `NOTICE` 文件的第三方 Apache-2.0 代码,须一并转发该 notice。

## 修订 —— 门控落地了,以及它查出了什么

`cargo-deny` 现在跑在 CI 里(`deny.toml`,以及 `ci.yml` 中的 `deny` job),而不再只是评审时手动
执行 —— 这是"政策"与"习惯"的区别。

首次运行拒绝了四个 crate。其中三个只是上面那份清单漏列的宽松许可,现补入:

| 许可 | crate | 为何允许 |
|---|---|---|
| `CDLA-Permissive-2.0` | `webpki-roots` | `egui_extras` 的 http feature 背后的根证书列表。宽松,无 copyleft。 |
| `LicenseRef-UFL-1.0` | `epaint_default_fonts` | egui 内置的 Ubuntu 字体。宽松,且该 crate 另有 `OFL-1.1` 与 `MIT OR Apache-2.0` 覆盖。 |
| `Apache-2.0 WITH LLVM-exception` | `target-lexicon` | 该例外严格比 `Apache-2.0` 更宽松;它被拒是因为表达式不是清单里已有的裸 `Apache-2.0`。 |

第四个是**真实的违规**:**`serialport`(mtty-ui 的直接依赖)是 `MPL-2.0`** —— 弱 copyleft,
本 ADR 明确排除。它从被引入那天起就被记录为 MIT,而这条错误声明被一路抄进 ADR 0037 与 manifest 注释,
无人核对。

**处理方式:替换该 crate,而不是给它开豁免。** 现在依赖是 `serial2`(BSD-2-Clause OR Apache-2.0),
同一项目的 fork。另一条路是豁免 MPL-2.0,已被否决:引擎是要能被第三方嵌入的,"引擎内不含 copyleft"
比省一个 crate 的麻烦更值钱。细节见 ADR 0037。

### 安全公告

同一次首跑还报出 1 个漏洞与 7 个停止维护的 crate。两者都在被写成"可接受"之前做了评估。

**该漏洞被判定为不可达,据此接受。** RUSTSEC-2023-0071(Marvin Attack)是 `rsa` **解密**路径上的
时序侧信道,且没有已修复的版本。mtty 只经由 `ssh-key` 触及 `rsa`,且只用于解析与重新编码私钥:
工作区只调用 `PrivateKey::from_openssh` 与 `to_openssh`,别无其他,也不存在任何 `sign`、`verify`
或 RSA 解密调用 —— `mtty-keys` 里唯一的 `decrypt` 是 PuTTY `.ppk` 自己的 AES-256-CBC。
该 crate 被链接进二进制,但那个有漏洞的操作**不在本代码的任何调用路径上**。`deny.toml` 把这条推理
连同日期与"升级 `ssh-key` 时重评"的提示一起记在例外旁边。

**停止维护的那些是告警,不是例外。**「停止维护」指的是上游不再开发某个 crate,并不等于该 crate
有问题。把它们静默忽略就等于藏起来 —— 本仓库里那个 MPL-2.0 依赖正是这样被漏掉的 —— 因此门控用
`-W unmaintained` 把它们降为告警:**每次运行都打印,但构建保持绿**。六个之中,`rustybuzz` 与
`ttf-parser` 确实会随二进制发布、且确实解析字体文件,是最值得盯的两个;而 `ttf-parser` 在同一技术栈
里的继任者 `skrifa` 已经在依赖树中,这项迁移属于上游。其余几个要么只在编译期,要么是维护者已声明
不需要再更新的完整版本。

「自己重写来消掉告警」这条路被考虑过并否决了。重写文字排版或字体解析,是**引入**漏洞而不是消除漏洞的
典型场合;而为了绕开一个本身没写错的 crate 去手写 RSA,只会更糟。
