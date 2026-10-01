# View 规则

一个 pane 的标签标题、图标与徽章由 *View 规则引擎* 从上下文推导
(设计见 [ADR 0007](./decisions/0007-view-rule-engine.zh-CN.md))。规则存放于
`~/.config/mtty/views.json`(JSON),也可在 **设置 → View rules** 中编辑,
并带实时预览。

```jsonc
{
  "rules": [
    {
      "name": "work repos",              // 仅供编辑器显示的标签
      "match": { "path": "~/work/**" },  // path|command|agent|host|file
      "alias": "work",                   // 供 {alias} 使用的短名
      "icon": { "name": "git-branch", "color": "#81a1c1" },
      "title": "{alias}:{folder}",
      "badge": "repo"                    // 可选
    },
    { "match": { "agent": "claude" }, "icon": { "name": "claude" } },
    { "match": {}, "title": "{folder} · {osc_title}" }  // 兜底
  ],
  "projects": [ { "path": "~/work/miao", "alias": "miao" } ],
  "worktree_suffix": true
}
```

## 匹配
- 规则是**有序列表**;第一条**所有已给出的子句都命中**的规则胜出。用 ↑/↓ 调整顺序。
- 子句:`path`(对工作目录做 glob,展开 `~`)、`command`(前台命令)、`agent`(精确名)、
  `host`、`file`。
- Glob:`*` 仅在单个路径段内匹配,`**` 跨 `/`,`?` 匹配一个字符;区分大小写。
- 空 `match`(`{}`)匹配一切 —— 用作兜底。

## 标题模板
变量:`{alias} {cwd} {folder} {user} {host} {agent} {branch} {command}
{title} {osc_title} {shell} {index} {file}`。未知变量渲染为空。

## 项目
`projects` 把路径映射为别名(最长前缀胜出),在没有规则给出 alias 时作为 `{alias}`
兜底。开启 `worktree_suffix: true` 时,`.worktrees/<name>` 检出会得到 `<alias>-<name>`。

## 图标
`icon` 接受内置 `name`、`emoji`,以及 `color`(`#rrggbb`)。名称未命中时依次回退到
emoji、再到纯色圆点。内置名称:

`folder`、`file`、`file-text`、`terminal`、`code`、`git-branch`、`git-commit`、
`github`、`globe`、`bug`、`flame`、`cpu`、`cloud`、`database`、`package`、
`box`、`coffee`、`heart`、`star`、`flag`、`zap`、`lock`、`search`、`settings`、
`user`、`home`、`bell`、`layers`、`claude`。

## 回退
没有命中规则也没有项目时,标签显示工作目录的目录名,其次为程序的 OSC 标题。
`views.json` 缺失或格式错误时退化为同样的回退,而不会导致启动失败。
