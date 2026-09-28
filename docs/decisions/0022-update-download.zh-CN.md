# ADR 0022 — 应用内更新下载与校验

> English (default): [`0022-update-download.md`](0022-update-download.md)

状态:已接受。

## 背景

ADR 0013 只做了版本比较。被提示"有新版本"的用户应当能不离开应用就取得并核对产物 —— 而且
核对不能信任下载路径。

## 决定

`miaotty-app/src/update.rs` 加一小块 UI:

- **清单。** `update-check-url` 可返回首行为版本的纯文本文档,或 JSON:
  `{ "version": "0.2.0", "artifacts": { "macos-aarch64": { "url": …, "sha256": … } } }`。
  `platform_key()` 把 OS+arch 映射到该键。两种形式的解析都有单测。
- **下载。** *Download Update* 按钮(设置里,以及面板动词)仅在清单提供本平台产物时可用;在
  worker 线程里用 `curl -fL --max-time 600 -o <tmp>` 拉取。
- **校验。** SHA-256 **在 crate 内**计算(无依赖的流式实现,FIPS 180-4),与清单里的十六进制
  摘要比对;不匹配则删除文件并报错。该实现既对标准向量自测,也在测试套件中于真实文件上与
  `shasum` 交叉校验。
- **落盘。** 校验通过的文件移入 `~/Downloads`(否则退回临时路径),状态行报告路径与"校验通过"。

## 后果

- 正常路径完全在应用内,且完整性校验独立于 curl。
- HTTPS 传输加固定摘要有助于发现损坏与低级篡改;**签名**校验(minisign/ed25519)仍是后续,
  它也是自动安装、以及真正替换运行中应用之前的最后一个缺口。
- 全部无依赖(`curl` + 自研哈希),符合 ADR 0006 关于引擎与应用不引入 copyleft、少加新 crate 的要求。
