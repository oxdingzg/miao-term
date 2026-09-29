# 发布

发布流水线如何工作、需要配置什么,以及更新链如何衔接。英文为默认;请保持
[`RELEASE.md`](RELEASE.md) 同步。

## 触发

推送 tag:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

[`.github/workflows/release.yml`](../.github/workflows/release.yml) 会在 macOS、Linux 与
Windows 上构建、按平台打包、可选签名、上传产物、生成**更新清单**,并创建 GitHub Release。

## 产物

| 平台 | 产物 | 说明 |
|------|------|------|
| macOS | `miaotty-macos-arm64.zip`(内含 `.app`) | ad-hoc 签名;配置 Apple secrets 后公证 |
| Linux | `miaotty-linux-x86_64.tar.gz`、`miaotty-linux-x86_64.AppImage`、`dist/*.deb` | AppImage 为尽力而为(`continue-on-error`) |
| Windows | `miaotty-windows-x86_64.zip`、`miaotty-<ver>-x86_64.msi` | MSI 为尽力而为(`continue-on-error`) |

配置签名后,每个产物旁边会生成 `.sig`。

## Secrets(全部可选)

| Secret | 用途 |
|--------|------|
| `APPLE_CERT_P12`、`APPLE_CERT_PASSWORD`、`APPLE_ID`、`APPLE_TEAM_ID`、`APPLE_APP_PASSWORD` | 对 macOS 构建 codesign + 公证 + staple |
| `WINDOWS_CERT_PFX`(base64)、`WINDOWS_CERT_PASSWORD` | 用 `signtool` 签 MSI |
| `MINISIGN_SECRET_KEY` | 用 `minisign` 为所有产物签名(无密码密钥) |

未配置时流水线仍产出可用的(未签名)产物,并打印其跳过的步骤。

### 生成 minisign 密钥

```sh
minisign -G -W -p minisign.pub -s minisign.key     # -W:无密码,供 CI 使用
gh secret set MINISIGN_SECRET_KEY < minisign.key   # 密钥文件内容
```

将**公钥**(`minisign.pub`)随 release notes / 仓库发布,并告知用户把其单行内容写入配置:

```toml
update-pubkey = "RWQ…"
```

## 更新清单

发布作业会写出 `latest.json` 并随 release 附上:

```json
{
  "version": "0.1.0",
  "artifacts": {
    "macos-aarch64": {
      "url": "https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/miaotty-macos-arm64.zip",
      "sha256": "…",
      "signature": "https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/miaotty-macos-arm64.zip.sig"
    },
    "linux-x86_64": { "url": "…AppImage", "sha256": "…" },
    "windows-x86_64": { "url": "…msi", "sha256": "…" }
  }
}
```

让应用指向它:

```toml
update-check-url = "https://github.com/oxdingzg/miao-term/releases/latest/download/latest.json"
```

此后 *设置 → 检查更新* 会报告版本,*下载更新* 会获取当前平台产物并校验其 SHA-256
(设置了 `update-pubkey` 时另校验 minisign 签名);macOS 上 *安装并重启* 会带回滚 helper 替换
应用包(ADR 0025)。

平台键与应用 `platform_key()` 一致:`macos-aarch64`、`macos-x86_64`、`linux-x86_64`
(另含 `linux-x86_64-deb`)、`windows-x86_64`。

## MSI(Windows)——已验证

WiX 模板已入库:[`miaotty-app/wix/main.wxs`](../miaotty-app/wix/main.wxs)(把 `miaotty.exe`
**与** `miaotty-cli.exe` 安装到 `%ProgramFiles%\miaotty\bin`,把该目录加入机器 `PATH`,并注册
卸载项)。用 `cargo wix --package miaotty-app` 构建;需要 WiX 3.x(choco `wixtoolset`),且必须
**在 `miaotty-app/` 目录内**运行 —— 模板以相对路径引用 `wix\License.rtf`。

已在真实 Windows 11 主机端到端验证(2026-09-29):构建 → `msiexec /i` → 两个二进制落盘 + PATH
条目 + "miaotty 0.0.0" 卸载项 → 已安装的 app 能启动 → `msiexec /x` 后目录被清除。

## 仍待完成

- **AppImage** 路径为尽力而为,尚未在真实安装场景验证。
- 自我替换仅 macOS;Windows/Linux 目前改为打开下载文件。
- macOS zip 名称目前带架构(`arm64`);若要做 Intel 构建,需要 `macos-x86_64` 及对应匹配。
