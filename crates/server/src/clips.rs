//! Turning a pasted link into something the in-app player can load.
//! Video files are never downloaded or stored: only the link.

use gym_core::api::ClipProvider;
use url::Url;

pub struct ParsedClip {
    pub url: String,
    pub provider: ClipProvider,
    pub embed_url: Option<String>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn parse(raw: &str) -> Result<ParsedClip, String> {
    let raw = raw.trim();
    let url = Url::parse(raw).map_err(|_| "That is not a web address".to_string())?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err("Only web links can be clips".into());
    }
    let host = url.host_str().unwrap_or_default().trim_start_matches("www.").trim_start_matches("m.").to_ascii_lowercase();
    let segs: Vec<&str> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();

    let youtube_id = match host.as_str() {
        "youtu.be" => segs.first().map(|s| s.to_string()),
        "youtube.com" | "youtube-nocookie.com" => match segs.as_slice() {
            ["watch"] => url.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.into_owned()),
            ["shorts", id, ..] | ["embed", id, ..] | ["live", id, ..] | ["v", id, ..] => Some(id.to_string()),
            _ => None,
        },
        _ => None,
    };
    if host.contains("youtu") {
        let id = youtube_id.filter(|id| valid_id(id)).ok_or("That YouTube link has no video in it")?;
        let start = url
            .query_pairs()
            .find(|(k, _)| k == "t" || k == "start")
            .and_then(|(_, v)| v.trim_end_matches('s').parse::<u32>().ok());
        let mut embed = format!("https://www.youtube-nocookie.com/embed/{id}?playsinline=1&rel=0&modestbranding=1");
        if let Some(s) = start {
            embed.push_str(&format!("&start={s}"));
        }
        return Ok(ParsedClip { url: format!("https://www.youtube.com/watch?v={id}"), provider: ClipProvider::Youtube, embed_url: Some(embed) });
    }

    if host == "instagram.com" {
        // /reel/CODE, /reels/CODE, /p/CODE, /tv/CODE
        if let [kind, code, ..] = segs.as_slice()
            && matches!(*kind, "reel" | "reels" | "p" | "tv")
            && valid_id(code)
        {
            let kind = if *kind == "reels" { "reel" } else { kind };
            return Ok(ParsedClip {
                url: format!("https://www.instagram.com/{kind}/{code}/"),
                provider: ClipProvider::Instagram,
                // Undocumented by Meta; may refuse to play. The sheet handles that.
                embed_url: Some(format!("https://www.instagram.com/{kind}/{code}/embed/")),
            });
        }
        return Err("That Instagram link is not a reel or a post".into());
    }

    Ok(ParsedClip { url: url.to_string(), provider: ClipProvider::Other, embed_url: None })
}

pub fn provider_str(p: ClipProvider) -> &'static str {
    match p {
        ClipProvider::Youtube => "youtube",
        ClipProvider::Instagram => "instagram",
        ClipProvider::Other => "other",
    }
}

pub fn provider_parse(s: &str) -> ClipProvider {
    match s {
        "youtube" => ClipProvider::Youtube,
        "instagram" => ClipProvider::Instagram,
        _ => ClipProvider::Other,
    }
}

/// Recompute the embed address from the stored link.
pub fn embed_for(url: &str) -> Option<String> {
    parse(url).ok().and_then(|p| p.embed_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_forms() {
        for u in [
            "https://www.youtube.com/watch?v=abcDEF12345",
            "https://youtu.be/abcDEF12345",
            "https://youtube.com/shorts/abcDEF12345",
            "https://m.youtube.com/watch?v=abcDEF12345&feature=share",
        ] {
            let p = parse(u).unwrap();
            assert_eq!(p.provider, ClipProvider::Youtube);
            assert!(p.embed_url.unwrap().starts_with("https://www.youtube-nocookie.com/embed/abcDEF12345?playsinline=1"), "{u}");
        }
        assert!(parse("https://youtu.be/abc?t=42s").unwrap().embed_url.unwrap().ends_with("&start=42"));
        assert!(parse("https://www.youtube.com/feed/library").is_err());
    }

    #[test]
    fn instagram_forms() {
        let p = parse("https://www.instagram.com/reel/C1a2B3c4D5e/?igsh=xyz").unwrap();
        assert_eq!(p.provider, ClipProvider::Instagram);
        assert_eq!(p.url, "https://www.instagram.com/reel/C1a2B3c4D5e/");
        assert_eq!(p.embed_url.unwrap(), "https://www.instagram.com/reel/C1a2B3c4D5e/embed/");
        assert!(parse("https://www.instagram.com/someone/").is_err());
    }

    #[test]
    fn refuses_odd_links() {
        assert!(parse("javascript:alert(1)").is_err());
        assert!(parse("not a link").is_err());
        assert!(parse("https://youtu.be/<script>").is_err());
        let other = parse("https://example.com/video").unwrap();
        assert_eq!(other.provider, ClipProvider::Other);
        assert!(other.embed_url.is_none());
    }
}
