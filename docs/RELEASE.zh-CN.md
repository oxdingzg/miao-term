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

仓库根目录保留 `minisign.pub`,release 作业也会把它作为附件上传,因此
`…/releases/download/<tag>/minisign.pub` 始终可用。告知用户把其单行内容写入配置:

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


## 验证安装包

两种安装包都已在真实硬件上检查(2026-09-29)。

**Windows MSI** —— 见上文(构建 → 安装 → PATH → 运行 → 卸载)。

**Linux `.deb` 与 AppImage**(Ubuntu 24.04):

```sh
cargo build --release -p miaotty-app -p miaotty-cli
cargo install cargo-deb --locked && cargo deb -p miaotty-app --no-build
sudo dpkg -i target/debian/miaotty_*_amd64.deb     # /usr/bin/miaotty{,-cli}
miaotty-cli ping                                   # 可运行;报错仅因无 host
```

AppImage 需要**真正的 256x256 图标** —— `appimagetool` 拒绝 1x1 占位图,且桌面文件必须带
`Icon=` 键 —— 因此流水线用 `scripts/make-icon.py`(仅标准库)生成图标。结果:
`miaotty-linux-x86_64.AppImage` 内含 `miaotty` 与 `miaotty-cli`,并可正常提取
(`--appimage-extract`)。

**未验证**:GUI 本体。该机器的 X 显示属于登录界面(无授权 cookie),而在 `xvfb` + 软件
Vulkan(lavapipe)下 `wgpu` 报 `Invalid surface`(离屏软件 Vulkan 的限制,非应用问题)。这需要
真实桌面会话或 GPU。

## 校验已签名的发布(用户)

下载产物、它的 `.sig`,以及发布的公钥(`minisign.pub`):

```sh
curl -fsSLO https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/miaotty-macos-arm64.zip
curl -fsSLO https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/miaotty-macos-arm64.zip.sig
curl -fsSLO https://github.com/oxdingzg/miao-term/releases/download/v0.1.0/minisign.pub
minisign -Vm miaotty-macos-arm64.zip -p minisign.pub      # -> "Signature and comment signature verified"
```

清单([`latest.json`](#更新清单))为每个产物同时提供 `sha256` 与 `signature` URL，可二者择一或都校验:

```sh
shasum -a 256 miaotty-macos-arm64.zip   # 与清单里的 "sha256" 比对
```

未配置 `MINISIGN_SECRET_KEY` 时不会签名，因此未签名的发布没有 `.sig` —— 校验是可选的，安装并不要求。

## 仍待完成

- **AppImage** 路径为尽力而为,尚未在真实安装场景验证。
- 自我替换仅 macOS;Windows/Linux 目前改为打开下载文件。
- macOS zip 名称目前带架构(`arm64`);若要做 Intel 构建,需要 `macos-x86_64` 及对应匹配。
