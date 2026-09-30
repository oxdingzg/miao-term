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

不发布 release 的演练:

```sh
gh workflow run release.yml --ref main -f tag=v0.0.1
```

手动运行执行相同的打包、签名与清单检查,上传汇总产物 `release-assembled`。
清单使用传入的 tag;只有正式发布该 tag 后,其中的下载 URL 才会可用。

## 产物

| 平台 | 产物 | 说明 |
|------|------|------|
| macOS | `miaotty-macos-arm64.zip`、`miaotty-macos-x86_64.zip`(各含 `miaotty.app` 与 `miaotty-native.app`) | ad-hoc 签名;配置 Apple secrets 后公证。Apple Silicon 用 `macos-latest`,Intel 用 `macos-15-intel` |
| Linux | `miaotty-linux-x86_64.tar.gz`、`miaotty-linux-x86_64.AppImage`、`dist/*.deb` | AppImage 含明确的 `AppRun` 入口 |
| Windows | `miaotty-windows-x86_64.zip`、`miaotty-app-<ver>-x86_64.msi` | 在 runner 上执行 MSI 安装/卸载验证 |

配置签名后,每个产物旁边会生成 `.sig`。
分离签名在平台代码签名完成后,由 Linux 汇总作业统一生成,并用已提交的公钥校验。
所有 app bundle、主应用压缩包与安装包均包含两个 host 和 CLI;
额外的 `miaotty-native-macos-*.zip` 则只包含独立 native host。
四个 runner 的构建与 AppImage/MSI 打包均必须成功。

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

## 备份与轮换 minisign 密钥

GitHub Secrets 是**只写**的:设了 `MINISIGN_SECRET_KEY` 之后,UI/API 都读不回原值。本地副本丢失
**不会**让发布中断(CI 照常签名),但你也拿不回原始私钥——所以要留离线备份,并把它当长期密钥:

- 公钥在仓库(`minisign.pub`)并随每次 release 附加。
- 私钥请另存到可靠处(密码管理器 / 加密副本 / macOS 钥匙串)。若要轮换:生成新密钥对 →
  `gh secret set MINISIGN_SECRET_KEY` → 替换 `minisign.pub` → 告知用户更新 `update-pubkey`。
  旧 release 仍可用归档的旧公钥校验。

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

平台键与应用 `platform_key()` 一致:`macos-aarch64`、`macos-x86_64`、`linux-x86_64`、
`linux-aarch64`、`windows-x86_64`。`linux-x86_64-deb` 只是 `.deb` 产物的清单专用键
(应用从不会请求它),而 `release.yml` 目前不产出 `linux-aarch64` 清单条目。

`scripts/build-update-manifest.py` 要求两种 macOS 架构、Linux、Windows 与 `.deb` 条目齐全,
从实际文件计算 SHA-256,优先选择 AppImage/MSI,其次才是压缩包。
配置 minisign 后缺少签名会使汇总作业失败;所有分离签名均用 `minisign.pub` 校验。
签名前用 `--collect-only` 把嵌套的 `dist/` 与 `target/wix/` 上传产物收集到发布目录;
重复文件名直接报错,避免安装包相互覆盖。
用 `python3 scripts/test-release-manifest.py` 检查产物选择及失败场景。

## MSI(Windows)——已验证

WiX 模板已入库:[`miaotty-app/wix/main.wxs`](../miaotty-app/wix/main.wxs)(把 `miaotty.exe`
、`miaotty-cli.exe` 与 `miaotty-native.exe` 安装到 `%ProgramFiles%\miaotty\bin`,把该目录加入机器 `PATH`,并注册
卸载项)。用 `cargo wix --package miaotty-app` 构建;需要 WiX 3.x(choco `wixtoolset`),且必须
**在 `miaotty-app/` 目录内**运行 —— 模板以相对路径引用 `wix\License.rtf`。

已在真实 Windows 11 主机端到端验证(2026-09-29):构建 → `msiexec /i` → 两个二进制落盘 + PATH
条目 + "miaotty 0.0.0" 卸载项 → 已安装的 app 能启动 → `msiexec /x` 后目录被清除。


## 验证安装包

两种安装包都已在真实硬件上检查(2026-09-29)。

**Windows MSI** —— 见上文(构建 → 安装 → PATH → 运行 → 卸载)。

**Linux `.deb` 与 AppImage**(Ubuntu 24.04):

```sh
cargo build --release -p miaotty-app -p miaotty-cli -p miao-term-widget
cargo install cargo-deb --locked && cargo deb -p miaotty-app --no-build
sudo dpkg -i target/debian/miaotty_*_amd64.deb     # /usr/bin/miaotty{,-cli,-native}
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

## 验收覆盖与剩余工作

发布工作流检查 Linux 包内容与 CLI 运行,以及 Windows MSI 安装 → 三个二进制 + URL handler
→ CLI 运行 → 卸载。这些检查不代表交互桌面体验已验收。上文的真机记录针对旧的双二进制包,
不代表新增 native host 的安装验证。

发布演练(2026-09-30,`16b2230`):
[运行 36663466348](https://github.com/oxdingzg/miao-term/actions/runs/36663466348)
的四个构建作业与汇总作业全部通过。下载 `release-assembled` 后独立复核:
五个清单条目的 SHA-256 全部匹配,九个分离签名全部通过;两种 macOS 压缩包均含两个 bundle,
每个 bundle 有三个二进制和图标;Windows zip 与 Linux tar/deb 均包含三个二进制。
清单为 Linux 选择 AppImage,为 Windows 选择 MSI。Windows runner 的安装/URL 注册/卸载检查通过。
本次为手动演练,未创建 GitHub Release。

同一 commit 的 macOS 本地验证:下载解压的两个 bundle 均通过
`codesign --verify --deep --strict`;fmt、严格 clippy、workspace 测试与四项 release 性能预算均通过。
下载包内的二进制经 MTP 驱动,两个 host 的 Kitty 内联图片及 Mermaid 时序图/饼图预览均已截图确认。
eframe 外壳在系统浅色主题下也保持深色,使预览文字可读。

本地收尾(2026-09-30,基于 `3357aab` 的工作区后续改动):两个 host 复用同一套标签/会话
右键菜单组件与分组边界规则,native 已绘制分组分隔线。“关闭标签”、标签 `×` 与会话行中键
关闭整个分屏标签;键盘关闭仍保留当前窗格行为。回归测试覆盖装饰字段持久化(含旧格式默认值)、
菜单可见性及分隔符位置、关闭/移动后的焦点、关闭下方标签,以及 egui 实际绘制的分组分隔线。
本地 fmt、workspace clippy/测试、四项 release 性能预算、隐私扫描与七项清单测试均通过。

另运行了 macOS 打包脚本,只重定向源/输出根目录,避免覆盖已有 `dist/` 产物。两个新构建的
bundle 均含三个二进制和图标,通过 `codesign --verify --deep --strict`(ad-hoc 签名),
CLI help 无 host 也可运行。这**不代表** Developer ID 签名/公证、GUI 交互验收或新一轮
四 runner 演练。在这一轮本地验证时,改动尚未提交/推送,也未经过新的远端发布工作流。
后续即使打包成功,下表中的桌面验收项也仍保留为待完成。

macOS 更新 helper 测试对含空格路径的临时 bundle 实际执行替换与缺失下载时的回滚,
重启命令由测试替身记录。压缩包选择测试拒绝 native-only 下载,并从双 bundle 包选择 `miaotty.app`。
这些是组件验证,不代表真实的下载/安装/重启验收。Windows zip 更新同时替换 native、eframe 与 CLI。

| 检查 | 验收标准 | 仍需环境 |
|------|----------|----------|
| AppImage 桌面启动 | 直接运行 AppImage,打开 pane,使用 MTP,正常退出 | Linux 桌面 |
| Wayland 热键 | 授权门户请求;应用无焦点时触发热键 | Wayland 桌面 |
| Windows IME,两个 host | 输入中日韩组合文本,检查候选框位置、提交/取消、切换分屏 | Windows 交互桌面 |
| 更新自替换 | 下载/校验新版,安装/重启,保留工作区;验证失败恢复 | 已安装的 macOS app、Windows MSI/zip、Linux AppImage |
| 视觉验收 | 截图检查 IME 组合文本及 Windows/Linux 图片/Mermaid 预览;macOS 图片/预览见上文 | 桌面会话 |
| 平台签名 | 检查分发产物的 Developer ID/公证与 MSI Authenticode | Apple/Windows 签名凭证 |

每项桌面检查记录 commit、安装包哈希、命令、结果与截图。
手动演练不会产生可访问的更新下载 URL;下载验收需要正式发布或专用测试清单服务器。
