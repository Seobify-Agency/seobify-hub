//! User-agent catalog: search engines, AI training crawlers, AI search/assistant fetchers,
//! SEO tools, social previewers and generic bots. Matching is case-insensitive substring
//! matching in catalog order (specific entries first), cached per distinct UA by the caller.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Search,
    AiTraining,
    AiAssistant,
    SeoTool,
    Social,
    OtherBot,
    Human,
}

impl Category {
    pub const BOTS: [Category; 6] = [Category::Search, Category::AiTraining, Category::AiAssistant, Category::SeoTool, Category::Social, Category::OtherBot];

    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Search => "search",
            Category::AiTraining => "ai_training",
            Category::AiAssistant => "ai_assistant",
            Category::SeoTool => "seo_tool",
            Category::Social => "social",
            Category::OtherBot => "other_bot",
            Category::Human => "human",
        }
    }

    pub fn from_name(s: &str) -> Option<Category> {
        Category::BOTS.iter().copied().chain([Category::Human]).find(|c| c.as_str() == s)
    }

    pub fn label(&self, lang: &str) -> &'static str {
        let en = lang == "en";
        match self {
            Category::Search => if en { "Search engines" } else { "Arama motorları" },
            Category::AiTraining => if en { "AI training" } else { "AI eğitim" },
            Category::AiAssistant => if en { "AI search / assistants" } else { "AI arama / asistan" },
            Category::SeoTool => if en { "SEO tools" } else { "SEO araçları" },
            Category::Social => if en { "Social" } else { "Sosyal" },
            Category::OtherBot => if en { "Other bots" } else { "Diğer botlar" },
            Category::Human => if en { "Human / unknown" } else { "İnsan / bilinmeyen" },
        }
    }
}

/// Verification family: which published IP lists and reverse-DNS suffixes prove identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    None,
    Google,
    GoogleUser,
    Bing,
    Yandex,
    Baidu,
    Apple,
    Yahoo,
    OpenAiBot,
    OpenAiSearch,
    OpenAiUser,
    PerplexityBot,
    PerplexityUser,
}

pub struct FamilySpec {
    /// Range source ids (see `verify::SOURCES`).
    pub sources: &'static [&'static str],
    /// Accepted reverse-DNS suffixes (forward-confirmed).
    pub dns: &'static [&'static str],
}

impl Family {
    pub fn as_str(&self) -> &'static str {
        match self {
            Family::None => "none",
            Family::Google => "google",
            Family::GoogleUser => "google_user",
            Family::Bing => "bing",
            Family::Yandex => "yandex",
            Family::Baidu => "baidu",
            Family::Apple => "apple",
            Family::Yahoo => "yahoo",
            Family::OpenAiBot => "openai_gptbot",
            Family::OpenAiSearch => "openai_search",
            Family::OpenAiUser => "openai_user",
            Family::PerplexityBot => "perplexity_bot",
            Family::PerplexityUser => "perplexity_user",
        }
    }

    pub fn spec(&self) -> FamilySpec {
        match self {
            Family::None => FamilySpec { sources: &[], dns: &[] },
            Family::Google => FamilySpec { sources: &["google-googlebot", "google-special"], dns: &["googlebot.com", "google.com"] },
            Family::GoogleUser => FamilySpec { sources: &["google-user-triggered", "google-user-triggered-google"], dns: &["google.com", "googleusercontent.com"] },
            Family::Bing => FamilySpec { sources: &["bing"], dns: &["search.msn.com"] },
            Family::Yandex => FamilySpec { sources: &[], dns: &["yandex.ru", "yandex.net", "yandex.com"] },
            Family::Baidu => FamilySpec { sources: &[], dns: &["baidu.com", "baidu.jp"] },
            Family::Apple => FamilySpec { sources: &["apple"], dns: &["applebot.apple.com"] },
            Family::Yahoo => FamilySpec { sources: &[], dns: &["crawl.yahoo.net"] },
            Family::OpenAiBot => FamilySpec { sources: &["openai-gptbot"], dns: &[] },
            Family::OpenAiSearch => FamilySpec { sources: &["openai-searchbot"], dns: &[] },
            Family::OpenAiUser => FamilySpec { sources: &["openai-chatgpt-user"], dns: &[] },
            Family::PerplexityBot => FamilySpec { sources: &["perplexity-bot"], dns: &[] },
            Family::PerplexityUser => FamilySpec { sources: &["perplexity-user"], dns: &[] },
        }
    }

    pub fn verifiable(&self) -> bool {
        let s = self.spec();
        !s.sources.is_empty() || !s.dns.is_empty()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotDef {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub family: Family,
    pub owner: &'static str,
    /// Lowercase substrings; any one matches.
    #[serde(skip)]
    pub tokens: &'static [&'static str],
    /// All of these must also be present.
    #[serde(skip)]
    pub requires: &'static [&'static str],
    /// None of these may be present.
    #[serde(skip)]
    pub excludes: &'static [&'static str],
}

const fn bot(id: &'static str, name: &'static str, category: Category, family: Family, owner: &'static str, tokens: &'static [&'static str]) -> BotDef {
    BotDef { id, name, category, family, owner, tokens, requires: &[], excludes: &[] }
}

use Category::*;

/// Catalog in priority order. `googlebot-smartphone` must precede `googlebot-desktop`.
pub static CATALOG: &[BotDef] = &[
    // --- Google
    bot("googlebot-image", "Googlebot-Image", Search, Family::Google, "Google", &["googlebot-image"]),
    bot("googlebot-video", "Googlebot-Video", Search, Family::Google, "Google", &["googlebot-video"]),
    bot("googlebot-news", "Googlebot-News", Search, Family::Google, "Google", &["googlebot-news"]),
    BotDef { id: "googlebot-smartphone", name: "Googlebot Smartphone", category: Search, family: Family::Google, owner: "Google", tokens: &["googlebot/", "googlebot "], requires: &["mobile"], excludes: &[] },
    bot("googlebot-desktop", "Googlebot Desktop", Search, Family::Google, "Google", &["googlebot/", "googlebot "]),
    bot("google-inspectiontool", "Google-InspectionTool", Search, Family::Google, "Google", &["google-inspectiontool"]),
    bot("google-cloudvertexbot", "Google-CloudVertexBot", AiTraining, Family::Google, "Google", &["google-cloudvertexbot"]),
    bot("googleother", "GoogleOther", Search, Family::Google, "Google", &["googleother"]),
    bot("adsbot-google", "AdsBot-Google", Search, Family::Google, "Google", &["adsbot-google"]),
    bot("mediapartners-google", "Mediapartners-Google", Search, Family::Google, "Google", &["mediapartners-google"]),
    bot("storebot-google", "Storebot-Google", Search, Family::Google, "Google", &["storebot-google"]),
    bot("google-user-fetcher", "Google user-triggered fetcher", Search, Family::GoogleUser, "Google", &["feedfetcher-google", "google-read-aloud", "google-site-verification", "googleproducer", "google-safety"]),
    // --- Microsoft / other search engines
    bot("adidxbot", "AdIdxBot", Search, Family::Bing, "Microsoft", &["adidxbot"]),
    bot("bingbot", "Bingbot", Search, Family::Bing, "Microsoft", &["bingbot/", "bingpreview", "microsoftpreview"]),
    bot("yandexbot", "YandexBot", Search, Family::Yandex, "Yandex", &["yandexbot", "yandeximages", "yandexmobilebot", "yandex.com/bots"]),
    bot("baiduspider", "Baiduspider", Search, Family::Baidu, "Baidu", &["baiduspider"]),
    // --- AI search / assistants (before AI training so "Claude-SearchBot" never falls through)
    bot("oai-searchbot", "OAI-SearchBot", AiAssistant, Family::OpenAiSearch, "OpenAI", &["oai-searchbot"]),
    bot("chatgpt-user", "ChatGPT-User", AiAssistant, Family::OpenAiUser, "OpenAI", &["chatgpt-user"]),
    bot("claude-searchbot", "Claude-SearchBot", AiAssistant, Family::None, "Anthropic", &["claude-searchbot"]),
    bot("claude-user", "Claude-User", AiAssistant, Family::None, "Anthropic", &["claude-user", "claude-web"]),
    bot("perplexity-user", "Perplexity-User", AiAssistant, Family::PerplexityUser, "Perplexity", &["perplexity-user"]),
    bot("perplexitybot", "PerplexityBot", AiAssistant, Family::PerplexityBot, "Perplexity", &["perplexitybot"]),
    bot("mistralai-user", "MistralAI-User", AiAssistant, Family::None, "Mistral AI", &["mistralai-user"]),
    bot("duckassistbot", "DuckAssistBot", AiAssistant, Family::None, "DuckDuckGo", &["duckassistbot"]),
    bot("meta-externalfetcher", "Meta-ExternalFetcher", AiAssistant, Family::None, "Meta", &["meta-externalfetcher"]),
    // --- AI training
    bot("gptbot", "GPTBot", AiTraining, Family::OpenAiBot, "OpenAI", &["gptbot"]),
    bot("claudebot", "ClaudeBot", AiTraining, Family::None, "Anthropic", &["claudebot"]),
    bot("anthropic-ai", "anthropic-ai", AiTraining, Family::None, "Anthropic", &["anthropic-ai"]),
    bot("ccbot", "CCBot", AiTraining, Family::None, "Common Crawl", &["ccbot"]),
    bot("bytespider", "Bytespider", AiTraining, Family::None, "ByteDance", &["bytespider"]),
    bot("amazonbot", "Amazonbot", AiTraining, Family::None, "Amazon", &["amazonbot"]),
    bot("meta-externalagent", "meta-externalagent", AiTraining, Family::None, "Meta", &["meta-externalagent"]),
    bot("cohere-ai", "cohere-ai", AiTraining, Family::None, "Cohere", &["cohere-ai", "cohere-training-data-crawler"]),
    bot("diffbot", "Diffbot", AiTraining, Family::None, "Diffbot", &["diffbot"]),
    bot("applebot-extended", "Applebot-Extended", AiTraining, Family::Apple, "Apple", &["applebot-extended"]),
    bot("ai2bot", "AI2Bot", AiTraining, Family::None, "Allen Institute for AI", &["ai2bot"]),
    // --- remaining search engines
    bot("applebot", "Applebot", Search, Family::Apple, "Apple", &["applebot"]),
    bot("duckduckbot", "DuckDuckBot", Search, Family::None, "DuckDuckGo", &["duckduckbot"]),
    bot("yeti", "Yeti (Naver)", Search, Family::None, "Naver", &["yeti/", "naver.me/spd"]),
    bot("seznambot", "SeznamBot", Search, Family::None, "Seznam", &["seznambot"]),
    bot("petalbot", "PetalBot", Search, Family::None, "Huawei", &["petalbot"]),
    bot("yahoo-slurp", "Yahoo! Slurp", Search, Family::Yahoo, "Yahoo", &["yahoo! slurp", "slurp"]),
    // --- SEO tools
    bot("ahrefsbot", "AhrefsBot", SeoTool, Family::None, "Ahrefs", &["ahrefsbot", "ahrefssiteaudit"]),
    bot("semrushbot", "SemrushBot", SeoTool, Family::None, "Semrush", &["semrushbot", "siteauditbot", "semrush"]),
    bot("mj12bot", "MJ12bot", SeoTool, Family::None, "Majestic", &["mj12bot"]),
    bot("dotbot", "DotBot", SeoTool, Family::None, "Moz", &["dotbot", "rogerbot"]),
    bot("screaming-frog", "Screaming Frog SEO Spider", SeoTool, Family::None, "Screaming Frog", &["screaming frog"]),
    bot("sitebulb", "Sitebulb", SeoTool, Family::None, "Sitebulb", &["sitebulb"]),
    bot("seobify", "Seobify crawler", SeoTool, Family::None, "Seobify", &["seobify"]),
    // --- social previewers
    bot("facebookexternalhit", "facebookexternalhit", Social, Family::None, "Meta", &["facebookexternalhit", "facebookcatalog"]),
    bot("telegrambot", "TelegramBot", Social, Family::None, "Telegram", &["telegrambot"]),
    bot("twitterbot", "Twitterbot", Social, Family::None, "X", &["twitterbot"]),
    bot("linkedinbot", "LinkedInBot", Social, Family::None, "LinkedIn", &["linkedinbot"]),
    bot("slackbot", "Slackbot", Social, Family::None, "Slack", &["slackbot", "slack-imgproxy"]),
    bot("whatsapp", "WhatsApp", Social, Family::None, "Meta", &["whatsapp"]),
    bot("discordbot", "Discordbot", Social, Family::None, "Discord", &["discordbot"]),
    bot("pinterestbot", "Pinterestbot", Social, Family::None, "Pinterest", &["pinterestbot", "pinterest/"]),
    // --- automation / generic
    bot("http-client", "HTTP library / headless browser", OtherBot, Family::None, "—", &["curl/", "wget/", "python-requests", "python-urllib", "aiohttp", "go-http-client", "okhttp", "apache-httpclient", "libwww-perl", "node-fetch", "axios/", "headlesschrome", "phantomjs", "java/", "scrapy"]),
    BotDef { id: "other-bot", name: "Other bot", category: OtherBot, family: Family::None, owner: "—", tokens: &["bot", "crawler", "spider", "crawl"], requires: &[], excludes: &["cubot"] },
];

/// Pseudo-entry for requests that match no bot.
pub static HUMAN: BotDef = bot("human", "Human / unknown", Human, Family::None, "—", &[]);

/// Classifies a user agent; `None` = human / unknown.
pub fn classify(ua: &str) -> Option<&'static BotDef> {
    if ua.is_empty() {
        return None;
    }
    let lower = ua.to_ascii_lowercase();
    CATALOG.iter().find(|d| d.tokens.iter().any(|t| lower.contains(t)) && d.requires.iter().all(|t| lower.contains(t)) && !d.excludes.iter().any(|t| lower.contains(t)))
}

pub fn find(id: &str) -> Option<&'static BotDef> {
    CATALOG.iter().find(|d| d.id == id).or_else(|| (id == "human").then_some(&HUMAN))
}

/// Bot ids counted as "Googlebot" in reports (the Google search crawlers proper).
pub fn is_googlebot(id: &str) -> bool {
    id.starts_with("googlebot-")
}

/// SQL list of Googlebot ids for `bot_id IN (…)` clauses.
pub fn googlebot_sql_list() -> String {
    CATALOG.iter().filter(|d| is_googlebot(d.id)).map(|d| format!("'{}'", d.id)).collect::<Vec<_>>().join(",")
}

/// SQL list of bot ids in a category.
pub fn category_sql_list(cat: Category) -> String {
    let ids: Vec<String> = CATALOG.iter().filter(|d| d.category == cat).map(|d| format!("'{}'", d.id)).collect();
    if ids.is_empty() { "''".into() } else { ids.join(",") }
}

/// SQL list of bot ids of a verification family.
pub fn family_sql_list(family: Family) -> String {
    let ids: Vec<String> = CATALOG.iter().filter(|d| d.family == family).map(|d| format!("'{}'", d.id)).collect();
    if ids.is_empty() { "''".into() } else { ids.join(",") }
}

/// Device class of a human request (aggregated only; never stored per request).
pub fn device_class(ua: &str) -> &'static str {
    if ua.is_empty() {
        return "other";
    }
    let l = ua.to_ascii_lowercase();
    if l.contains("ipad") || l.contains("tablet") || (l.contains("android") && !l.contains("mobile")) {
        "tablet"
    } else if l.contains("mobi") || l.contains("iphone") || l.contains("android") {
        "mobile"
    } else if l.contains("windows") || l.contains("macintosh") || l.contains("x11") || l.contains("cros") || l.contains("linux") {
        "desktop"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(ua: &str) -> &'static str {
        classify(ua).map(|d| d.id).unwrap_or("human")
    }

    #[test]
    fn search_engines() {
        assert_eq!(id("Mozilla/5.0 (Linux; Android 6.0.1; Nexus 5X Build/MMB29P) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.6668.70 Mobile Safari/537.36 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"), "googlebot-smartphone");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Googlebot/2.1; +http://www.google.com/bot.html) Chrome/129.0.6668.70 Safari/537.36"), "googlebot-desktop");
        assert_eq!(id("Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"), "googlebot-desktop");
        assert_eq!(id("Googlebot-Image/1.0"), "googlebot-image");
        assert_eq!(id("Googlebot-Video/1.0"), "googlebot-video");
        assert_eq!(id("Googlebot-News"), "googlebot-news");
        assert_eq!(id("Mozilla/5.0 (Linux; Android 6.0.1; Nexus 5X Build/MMB29P) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Mobile Safari/537.36 (compatible; Google-InspectionTool/1.0;)"), "google-inspectiontool");
        assert_eq!(id("Mozilla/5.0 (compatible; GoogleOther)"), "googleother");
        assert_eq!(id("Mozilla/5.0 (iPhone; CPU iPhone OS 14_7_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/14.1.2 Mobile/15E148 Safari/604.1 (compatible; AdsBot-Google-Mobile; +http://www.google.com/mobile/adsbot.html)"), "adsbot-google");
        assert_eq!(id("Mediapartners-Google"), "mediapartners-google");
        assert_eq!(id("Mozilla/5.0 (X11; Linux x86_64; Storebot-Google/1.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/79.0.3945.88 Safari/537.36"), "storebot-google");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm) Chrome/116.0.1938.76 Safari/537.36"), "bingbot");
        assert_eq!(id("Mozilla/5.0 (compatible; adidxbot/2.0; +http://www.bing.com/bingbot.htm)"), "adidxbot");
        assert_eq!(id("Mozilla/5.0 (compatible; YandexBot/3.0; +http://yandex.com/bots)"), "yandexbot");
        assert_eq!(id("Mozilla/5.0 (compatible; Baiduspider/2.0; +http://www.baidu.com/search/spider.html)"), "baiduspider");
        assert_eq!(id("DuckDuckBot/1.1; (+http://duckduckgo.com/duckduckbot.html)"), "duckduckbot");
        assert_eq!(id("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/13.1.1 Safari/605.1.15 (Applebot/0.1; +http://www.apple.com/go/applebot)"), "applebot");
        assert_eq!(id("Mozilla/5.0 (compatible; Yeti/1.1; +https://naver.me/spd)"), "yeti");
        assert_eq!(id("Mozilla/5.0 (compatible; SeznamBot/4.0; +https://o-seznam.cz/napoveda/vyhledavani/en/seznambot-crawler/)"), "seznambot");
        assert_eq!(id("Mozilla/5.0 (compatible;PetalBot;+https://webmaster.petalsearch.com/site/petalbot)"), "petalbot");
    }

    #[test]
    fn ai_training() {
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; GPTBot/1.1; +https://openai.com/gptbot"), "gptbot");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; ClaudeBot/1.0; +claudebot@anthropic.com)"), "claudebot");
        assert_eq!(id("anthropic-ai"), "anthropic-ai");
        assert_eq!(id("CCBot/2.0 (https://commoncrawl.org/faq/)"), "ccbot");
        assert_eq!(id("Mozilla/5.0 (Linux; Android 5.0) AppleWebKit/537.36 (KHTML, like Gecko) Mobile Safari/537.36 (compatible; Bytespider; spider-feedback@bytedance.com)"), "bytespider");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Amazonbot/0.1; +https://developer.amazon.com/support/amazonbot) Chrome/119.0.6045.214 Safari/537.36"), "amazonbot");
        assert_eq!(id("meta-externalagent/1.1 (+https://developers.facebook.com/docs/sharing/webmasters/crawler)"), "meta-externalagent");
        assert_eq!(id("cohere-training-data-crawler (+https://cohere.com)"), "cohere-ai");
        assert_eq!(id("Mozilla/5.0 (compatible; Diffbot/0.1; +http://www.diffbot.com)"), "diffbot");
        assert_eq!(id("Mozilla/5.0 (compatible; Applebot-Extended/0.1; +http://www.apple.com/go/applebot)"), "applebot-extended");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Google-CloudVertexBot; +https://cloud.google.com/enterprise-search)"), "google-cloudvertexbot");
    }

    #[test]
    fn ai_assistants() {
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; OAI-SearchBot/1.0; +https://openai.com/searchbot"), "oai-searchbot");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ChatGPT-User/1.0; +https://openai.com/bot"), "chatgpt-user");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Claude-SearchBot/1.0; +Claude-SearchBot@anthropic.com)"), "claude-searchbot");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Claude-User/1.0; +Claude-User@anthropic.com)"), "claude-user");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; PerplexityBot/1.0; +https://perplexity.ai/perplexitybot)"), "perplexitybot");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Perplexity-User/1.0; +https://perplexity.ai/perplexity-user)"), "perplexity-user");
        assert_eq!(id("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; MistralAI-User/1.0; +https://docs.mistral.ai/robots)"), "mistralai-user");
        assert_eq!(id("DuckAssistBot/1.2; (+http://duckduckgo.com/duckassistbot.html)"), "duckassistbot");
        assert_eq!(id("meta-externalfetcher/1.1 (+https://developers.facebook.com/docs/sharing/webmasters/crawler)"), "meta-externalfetcher");
    }

    #[test]
    fn seo_social_other_human() {
        assert_eq!(id("Mozilla/5.0 (compatible; AhrefsBot/7.0; +http://ahrefs.com/robot/)"), "ahrefsbot");
        assert_eq!(id("Mozilla/5.0 (compatible; SemrushBot/7~bl; +http://www.semrush.com/bot.html)"), "semrushbot");
        assert_eq!(id("Mozilla/5.0 (compatible; MJ12bot/v1.4.8; http://mj12bot.com/)"), "mj12bot");
        assert_eq!(id("Mozilla/5.0 (compatible; DotBot/1.2; +https://opensiteexplorer.org/dotbot; help@moz.com)"), "dotbot");
        assert_eq!(id("Screaming Frog SEO Spider/20.0"), "screaming-frog");
        assert_eq!(id("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Sitebulb/7.0"), "sitebulb");
        assert_eq!(id("Mozilla/5.0 (compatible; SeobifySpider/1.0; +https://seobify.com)"), "seobify");
        assert_eq!(id("facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)"), "facebookexternalhit");
        assert_eq!(id("Twitterbot/1.0"), "twitterbot");
        assert_eq!(id("LinkedInBot/1.0 (compatible; Mozilla/5.0; Apache-HttpClient +http://www.linkedin.com)"), "linkedinbot");
        assert_eq!(id("Slackbot-LinkExpanding 1.0 (+https://api.slack.com/robots)"), "slackbot");
        assert_eq!(id("WhatsApp/2.23.20.0"), "whatsapp");
        assert_eq!(id("TelegramBot (like TwitterBot)"), "telegrambot");
        assert_eq!(id("Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)"), "discordbot");
        assert_eq!(id("curl/8.4.0"), "http-client");
        assert_eq!(id("python-requests/2.31.0"), "http-client");
        assert_eq!(id("Mozilla/5.0 (compatible; MegaIndex.ru/2.0; +http://megaindex.com/crawler)"), "other-bot");
        assert_eq!(id("SomeNewSpider/0.3"), "other-bot");
        assert_eq!(id("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"), "human");
        assert_eq!(id("Mozilla/5.0 (Linux; Android 10; CUBOT X30) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Mobile Safari/537.36"), "human");
        assert_eq!(id(""), "human");
    }

    #[test]
    fn every_category_has_entries_and_ids_are_unique() {
        for cat in Category::BOTS {
            assert!(CATALOG.iter().filter(|d| d.category == cat).count() >= 2, "{cat:?}");
            assert_eq!(Category::from_name(cat.as_str()), Some(cat));
        }
        let mut ids: Vec<&str> = CATALOG.iter().map(|d| d.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
        assert!(googlebot_sql_list().contains("'googlebot-smartphone'"));
        assert!(Family::Google.verifiable() && !Family::None.verifiable());
    }

    #[test]
    fn devices() {
        assert_eq!(device_class("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile/15E148"), "mobile");
        assert_eq!(device_class("Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X)"), "tablet");
        assert_eq!(device_class("Mozilla/5.0 (Linux; Android 13; SM-X700) AppleWebKit/537.36 Safari/537.36"), "tablet");
        assert_eq!(device_class("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"), "desktop");
        assert_eq!(device_class(""), "other");
    }
}
