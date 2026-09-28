# ADR 0006 — 许可与依赖策略

> English (default): [`0006-license-policy.md`](0006-license-policy.md)

状态:已接受。

## 背景

引擎(`miao-term-*`)要能被第三方嵌入,应用(`miaotty-app`)是它的第一个消费者。
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
