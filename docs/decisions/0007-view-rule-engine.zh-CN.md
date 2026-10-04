# ADR 0007 — View 规则引擎

> English (default): [`0007-view-rule-engine.md`](0007-view-rule-engine.md)

状态:已接受。

## 背景

一个 pane 的身份应当**由上下文推导**,而不是手输。给定工作目录、正在运行的命令、
agent、host 或文件,应用应能知道该 pane 的短名(alias)、图标、标签标题、徽章,
以及该显示哪些 Details 组件。这套映射就是 *View 规则引擎*,它是标签外观、
Details 面板与 Open Quickly(U3)共同的数据源,因此要先于它们落地(roadmap U2)。

## 决定

单一 **规则集** 从 `~/.config/miaotty/views.json` 加载(JSON,便于用户编辑与往返
读写)。它位于 `mtty-config::view`(纯数据 + 匹配,不依赖终端内核):

```jsonc
{
  "rules": [
    {
      "name": "work repo",                 // 仅供编辑器显示的标签
      "match": { "path": "~/work/**" },    // path|command|agent|host|file,glob
      "alias": "work",                     // 供 {alias} 使用的短名
      "icon": { "name": "git-branch", "color": "#81a1c1" }, // 或 { "emoji": "🌿" }
      "title": "{alias} · {folder}",       // 模板,见变量
      "badge": "repo"                      // 标签徽章键(可选)
    },
    { "match": { "agent": "claude" }, "icon": { "name": "claude" } },
    { "match": {}, "title": "{osc_title}" } // `any` 兜底
  ],
  "projects": [ { "path": "~/work/miao", "alias": "miao" } ],
  "worktree_suffix": true
}
```

- **匹配维度**:`path`(对 cwd 做 glob,展开 `~`)、`command`(对前台命令做 glob)、
  `agent`(精确名)、`host`(glob)、`file`(对 pane 当前文件做 glob)。`match` 为空的
  规则匹配一切(`any` 兜底)。
- **求值**:规则是有序列表;第一条**所有已给出的匹配器都命中**的规则胜出
  (`move_up`/`move_down` 调整顺序)。单次遍历,O(规则 × 匹配器)。
- **标题模板**:`{alias} {cwd} {folder} {user} {host} {agent} {branch} {command}
  {title} {shell} {index} {file}`,外加 `{osc_title}` 取程序原始标题。未知变量
  渲染为空。`folder` 为路径最后一段;`cwd` 做缩写(家目录用 `~`)。
- **图标**:可以是内置图标名(由 UI 从仓库内手工绘制的图元集解析,不使用 SVG)、emoji,
  或纯色。引擎只携带描述符。
- **projects**:`路径 → 别名`,按最长前缀匹配;开启 `worktree_suffix` 且 cwd 位于
  `.worktrees/<name>` 检出时,别名为其加上该后缀。
- **持久化**:启动读取、变更写入;文件缺失或格式错误时回退为空规则集
  (即普通的终端标题)。

## 后果

- 标签 / Details / Open Quickly 都消费同一次求值得到的 `View`,因此保持一致,
  且无需渲染器即可测试。
- `views.json` 损坏时退化为 OSC 标题,而不是让启动失败。
- 图标集本体(29 个手工绘制的图元,位于 `miaotty-app/src/icons.rs`)与规则 *编辑器* UI
  属后续工作;本 ADR 覆盖模型与它们所依赖的匹配语义。
- Update:图标集与规则 *编辑器* UI 现已存在(可新增、调序、删除并保存规则)。
