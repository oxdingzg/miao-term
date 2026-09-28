//! Minimal English/Chinese UI strings (ADR 0013).
//!
//! English source strings are the keys; `t(Lang::En, key)` returns the key
//! unchanged, so untranslated strings degrade to English. Only the visible
//! chrome is translated, not log lines.

/// A UI language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        if s.starts_with("zh") || s.contains("chinese") || s.contains("中文") {
            Some(Lang::Zh)
        } else if s.starts_with("en") || s.contains("english") {
            Some(Lang::En)
        } else {
            None
        }
    }

    /// Parse a config value, else detect from `$LANG`, defaulting to English.
    pub fn resolve(config: Option<&str>) -> Self {
        config
            .and_then(Lang::parse)
            .or_else(|| std::env::var("LANG").ok().and_then(|l| Lang::parse(&l)))
            .unwrap_or(Lang::En)
    }
}

/// Translate a key. Unknown keys fall back to the key itself.
pub fn t(lang: Lang, key: &'static str) -> &'static str {
    match lang {
        Lang::En => key,
        Lang::Zh => zh(key),
    }
}

fn zh(key: &'static str) -> &'static str {
    match key {
        "Settings" => "设置",
        "Font size" => "字号",
        "Font family" => "字体族",
        "Opacity" => "透明度",
        "Line height" => "行高",
        "Cursor" => "光标",
        "Theme" => "主题",
        "Agents" => "Agent",
        "Notify" => "通知",
        "Keep awake" => "防休眠",
        "Badges" => "徽章",
        "Sleep guard" => "休眠保护",
        "Save to config.toml" => "保存到 config.toml",
        "Info" => "信息",
        "Agent" => "Agent",
        "Outline" => "大纲",
        "Git" => "Git",
        "Files" => "文件",
        "Ports" => "端口",
        "Queue" => "队列",
        "TABS" => "标签",
        "FILES" => "文件",
        "Compose" => "撰写",
        "Send" => "发送",
        "Queue it" => "入队",
        "No update URL configured" => "未配置更新地址",
        "Copy Path" => "复制路径",
        "Reveal in Finder" => "在访达中显示",
        "Composer" => "Composer",
        "Quick Terminal" => "快速终端",
        "Save Recipe" => "保存配方",
        "Open Recipe" => "打开配方",
        "New Tab" => "新建标签",
        "Split Right" => "向右分屏",
        "Split Down" => "向下分屏",
        "Close Tab" => "关闭标签",
        "Toggle Details" => "切换详情",
        "Find" => "查找",
        "Next Tab" => "下一个标签",
        "Previous Tab" => "上一个标签",
        "Check for Updates" => "检查更新",
        "New SSH Session" => "新建 SSH 会话",
        "New SSH Session…" => "新建 SSH 会话…",
        "Connect" => "连接",
        "Close" => "关闭",
        "Reload" => "重新加载",
        "Save" => "保存",
        "Preview" => "预览",
        "Edit" => "编辑",
        "Raw" => "原文",
        "Markdown" => "Markdown",
        "AGENT INTEGRATIONS" => "Agent 集成",
        "detected" => "已检测到",
        "not found" => "未找到",
        "Install hook" => "安装 hook",
        "Copy snippet" => "复制片段",
        "Launch" => "启动",
        "Snippet copied" => "片段已复制",
        "Open Externally" => "外部打开",
        "Edit in Tab" => "在标签中编辑",
        "Name" => "名称",
        "No recipes yet" => "暂无配方",
        "No listening ports" => "无监听端口",
        "Not a git repository" => "不是 git 仓库",
        "Clean" => "干净",
        "No agent in this pane" => "此 pane 无 agent",
        "Prompts send when the agent is idle" => "agent 空闲时自动发送",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_detects() {
        assert_eq!(Lang::parse("zh_CN.UTF-8"), Some(Lang::Zh));
        assert_eq!(Lang::parse("en-US"), Some(Lang::En));
        assert_eq!(Lang::parse("fr_FR"), None);
    }

    #[test]
    fn translation_falls_back_to_key() {
        assert_eq!(t(Lang::Zh, "Settings"), "设置");
        assert_eq!(t(Lang::Zh, "Untranslated Thing"), "Untranslated Thing");
        assert_eq!(t(Lang::En, "Settings"), "Settings");
    }
}
