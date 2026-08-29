use crate::model::Category;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Which field of a window a rule matches against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Field {
    /// Match against the application id (WM_CLASS / app_id).
    App,
    /// Match against the window title.
    Title,
}

impl Field {
    pub fn key(self) -> &'static str {
        match self {
            Field::App => "app",
            Field::Title => "title",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "app" => Some(Field::App),
            "title" => Some(Field::Title),
            _ => None,
        }
    }
}

/// A user-visible categorisation rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub id: Option<i64>,
    /// Regex, matched case-insensitively.
    pub pattern: String,
    pub field: Field,
    pub category: Category,
    /// Higher wins. Built-in rules use 10 (app) and 20 (title); user rules
    /// default to 100 so they always override the built-ins.
    #[serde(default = "default_priority")]
    pub priority: i32,
}

fn default_priority() -> i32 {
    100
}

impl Rule {
    fn new(pattern: &str, field: Field, category: Category, priority: i32) -> Self {
        Self {
            id: None,
            pattern: pattern.to_string(),
            field,
            category,
            priority,
        }
    }
}

struct CompiledRule {
    id: Option<i64>,
    pattern: String,
    re: Regex,
    field: Field,
    category: Category,
    priority: i32,
}

/// A compiled, priority-ordered set of rules.
pub struct RuleSet {
    rules: Vec<CompiledRule>,
}

impl RuleSet {
    pub fn compile(rules: &[Rule]) -> Result<Self, regex::Error> {
        let mut compiled = Vec::with_capacity(rules.len());
        for r in rules {
            compiled.push(CompiledRule {
                id: r.id,
                pattern: r.pattern.clone(),
                re: Regex::new(&format!("(?i)^({})$", r.pattern))?,
                field: r.field,
                category: r.category,
                priority: r.priority,
            });
        }
        // Highest priority first; ties keep insertion order (stable sort).
        compiled.sort_by_key(|r| std::cmp::Reverse(r.priority));
        Ok(Self { rules: compiled })
    }

    /// An empty ruleset — everything is uncategorised.
    pub fn empty() -> Self {
        Self { rules: Vec::new() }
    }

    /// Categorise a window. Rules are checked in priority order; the first
    /// match wins.
    pub fn categorize(&self, app_id: &str, title: &str) -> Category {
        for r in &self.rules {
            let hay = match r.field {
                Field::App => app_id,
                Field::Title => title,
            };
            if r.re.is_match(hay) {
                return r.category;
            }
        }
        Category::Uncategorised
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Number of rules (for status displays).
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Introspection: (id, pattern, field, category) for each rule.
    pub fn describe(&self) -> Vec<(Option<i64>, String, Field, Category)> {
        self.rules
            .iter()
            .map(|r| (r.id, r.pattern.clone(), r.field, r.category))
            .collect()
    }
}

/// Sensible built-in rules for a Linux desktop, tuned for KDE Plasma first.
///
/// App rules use anchored alternation (priority 10). A few high-priority
/// title rules (priority 20) catch video streaming inside browsers, because
/// "Netflix in Firefox" is more accurately Media than Browsers.
pub fn default_rules() -> Vec<Rule> {
    let mut v: Vec<Rule> = Vec::new();

    let app = |pattern: &str, cat: Category| Rule::new(pattern, Field::App, cat, 10);

    // Work & coding
    for p in [
        "code|code-oss|code-insiders|vscodium|cursor|windsurf",
        "jetbrains-.*|idea|idea64|pycharm|webstorm|clion|goland|rider|datagrip|rustrover|android-studio",
        "nvim|vim|neovim|emacs|sublime_text|kate|gedit|lapce|helix|zed",
        "libreoffice.*|onlyoffice-desktopeditors|calligrawords|wps-office",
        "postman|insomnia|beekeeper-studio|docker-desktop|virt-manager|godot",
    ] {
        v.push(app(p, Category::Work));
    }

    // Browsers (incl. common PWA host browsers)
    v.push(app(
        "firefox|firefox-esr|librewolf|floorp|zen-beta|zen-alpha|chromium|chromium-browser|google-chrome|chrome|brave|brave-browser|vivaldi-stable|vivaldi|opera|epiphany|gnome-web|falkon|thorium-browser|appimagepool.*browser",
        Category::Browsers,
    ));

    // Communication
    v.push(app(
        "discord|discord-ptb|discord-canary|vesktop|webcord|telegramdesktop|telegram-desktop|org.telegram.desktop|slack|signal|whatsapp.*|teams|teams-for-linux|element|fractal|thunderbird|betterbird|kmail|kontact|claws-mail|hexchat|irc-client",
        Category::Communication,
    ));

    // Media & streaming
    v.push(app(
        "vlc|mpv|mpv-wrapper|celluloid|haruna|totem|smplayer|spotify|spotify-client|elisa|lollypop|rhythmbox|audacious|sayonara|amberol|nicotine\\+|nicotine-plus",
        Category::Media,
    ));

    // Creative & design
    v.push(app(
        "gimp|gimp-.*|inkscape|krita|blender|blenderplayer|davinci-resolve|resolve|figma.*|audacity|ardour|obs|com\\.obsproject\\.Studio|obs-studio|kdenlive|pitivi|openshot|darktable|rawtherapee|vectorpea|vpea",
        Category::Creative,
    ));

    // Games
    v.push(app(
        "steam|steamapp.*|steam_.*|lutris|wine|wine64|wineserver|gamescope|heroic.*|prismlauncher|minecraft|polymc|bottles|moonlight|sunshine|yuzu|ryujinx|citra|dolphin-emu|ppsspp|pcsx2|rpcs3",
        Category::Gaming,
    ));

    // System, files, terminals, settings
    v.push(app(
        "plasmashell|kwin_wayland|kwin_x11|kded6|systemsettings|kinfocenter|drkonqi|plasma.*|org\\.kde\\.plasma.*",
        Category::System,
    ));
    v.push(app(
        "dolphin|nautilus|org\\.gnome\\.Nautilus|thunar|nemo|pcmanfm|konqueror|krusader|ark|filelight|gwenview|okular|eog|loupe|spectacle|flameshot|discover|pamac|octopi|yad|gparted|partitionmanager|baobab|skanpage|simple-scan",
        Category::System,
    ));
    v.push(app(
        "konsole|yakuake|kitty|alacritty|foot|footclient|wezterm|gnome-terminal-.*|kgx|xfce4-terminal|terminator|tilix|xterm|st|urxvt|sakura",
        Category::System,
    ));

    // Title rules that refine browser usage into Media (streaming detection).
    // NOTE: the regex crate has no look-ahead, so "YouTube but not Studio"
    // is expressed as a higher-priority Work rule that shadows the Media one.
    v.push(Rule::new(
        "youtube.*studio.*|youtube.*-.*studio",
        Field::Title,
        Category::Work,
        25,
    ));
    v.push(Rule::new(
        "netflix.*|hbo max.*|disney\\+.*|prime video.*|amazon prime video.*|youtube.*|twitch.*|crunchyroll.*",
        Field::Title,
        Category::Media,
        20,
    ));

    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rs() -> RuleSet {
        RuleSet::compile(&default_rules()).unwrap()
    }

    #[test]
    fn categorises_common_apps() {
        let r = rs();
        assert_eq!(r.categorize("firefox", "whatever"), Category::Browsers);
        assert_eq!(r.categorize("Google-chrome", "x"), Category::Browsers);
        assert_eq!(r.categorize("jetbrains-idea", "x"), Category::Work);
        assert_eq!(
            r.categorize("code", "x — Visual Studio Code"),
            Category::Work
        );
        assert_eq!(r.categorize("steam", "Steam"), Category::Gaming);
        assert_eq!(r.categorize("dolphin", "Home — Dolphin"), Category::System);
        assert_eq!(r.categorize("konsole", "zsh"), Category::System);
        assert_eq!(
            r.categorize("totally-unknown-app", "hi"),
            Category::Uncategorised
        );
    }

    #[test]
    fn title_rules_override_browser_app_rule() {
        let r = rs();
        // Netflix in Firefox should count as Media, not Browsers.
        assert_eq!(
            r.categorize("firefox", "Netflix — Watch TV Shows Online"),
            Category::Media
        );
        assert_eq!(
            r.categorize("firefox", "GitHub · Build software"),
            Category::Browsers
        );
    }

    #[test]
    fn user_rules_override_builtin() {
        let mut rules = default_rules();
        // Terminal is System by default; the user wants it to be Work.
        rules.push(Rule::new("konsole|kitty", Field::App, Category::Work, 100));
        let r = RuleSet::compile(&rules).unwrap();
        assert_eq!(r.categorize("kitty", "zsh"), Category::Work);
    }

    #[test]
    fn empty_ruleset_uncategorises() {
        let r = RuleSet::empty();
        assert_eq!(r.categorize("firefox", "x"), Category::Uncategorised);
    }
}
