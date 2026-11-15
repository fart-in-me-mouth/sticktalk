//! Command-line launches and `sticktalk://` links.
use crate::address::Server;
use sticktalk_desktop_platform::activation::Activation;

#[derive(Debug, PartialEq, Eq)]
pub struct Launch {
    /// `--smoke <url>`: hidden load check, see `README.md`.
    pub smoke: Option<String>,
    /// `--smoke-eval <js>`: with `--smoke`, run this in the first page.
    pub smoke_eval: Option<String>,
    /// `--smoke-wait <ms>`: how long the page must be quiet before the report.
    pub smoke_wait: Option<u64>,
    /// `--smoke-shot <png>`: with `--smoke`, also save what the page rendered.
    pub smoke_shot: Option<std::path::PathBuf>,
    /// `--smoke-root <dir>`: data folder for a smoke run that others can join.
    pub smoke_root: Option<std::path::PathBuf>,
    /// The previous client's autostart entry passes `--background`.
    pub background: bool,
    pub activation: Activation,
}

pub fn launch(args: impl IntoIterator<Item = String>) -> Launch {
    let mut launch = Launch {
        smoke: None,
        smoke_eval: None,
        smoke_wait: None,
        smoke_shot: None,
        smoke_root: None,
        background: false,
        activation: Activation::Open,
    };
    let mut args = args.into_iter().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke" => launch.smoke = args.next(),
            "--smoke-eval" => launch.smoke_eval = args.next(),
            "--smoke-wait" => launch.smoke_wait = args.next().and_then(|ms| ms.parse().ok()),
            "--smoke-shot" => launch.smoke_shot = args.next().map(Into::into),
            "--smoke-root" => launch.smoke_root = args.next().map(Into::into),
            "--background" => launch.background = true,
            // Windows passes the clicked link as `"%1"`. A malformed link still
            // opens the app; it just doesn't go anywhere.
            link if link.starts_with("sticktalk:") => {
                launch.activation = Activation::parse(link).unwrap_or(Activation::Open)
            }
            _ => {}
        }
    }
    launch
}

/// The web app page a link points at, or `None` when focusing the window is
/// all there is to do. These are the web app's own URL forms: its "Copy Link"
/// for invites is `<origin>/?invite=<code>` and it opens `?channel=<id>`.
pub fn target(server: &Server, activation: &Activation) -> Option<String> {
    let mut url = server.home();
    match activation {
        Activation::Open => return None,
        Activation::Invite(code) => url.query_pairs_mut().append_pair("invite", code),
        // The web app has no address for a single message; open its channel.
        Activation::Message { channel_id, .. } => url
            .query_pairs_mut()
            .append_pair("channel", &channel_id.to_string()),
    };
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::parse;

    fn args(list: &[&str]) -> Launch {
        launch(
            std::iter::once("sticktalk.exe")
                .chain(list.iter().copied())
                .map(String::from),
        )
    }

    #[test]
    fn links_map_to_the_web_apps_own_urls() {
        let server = parse("https://chat.example.com").unwrap();
        let target = |link: &str| target(&server, &Activation::parse(link).unwrap());
        assert_eq!(target("sticktalk://open"), None);
        assert_eq!(
            target("sticktalk://invite/Abc-123_x").as_deref(),
            Some("https://chat.example.com/?invite=Abc-123_x")
        );
        assert_eq!(
            target("sticktalk://message/42/9001").as_deref(),
            Some("https://chat.example.com/?channel=42")
        );
        let local = parse("http://127.0.0.1:8080").unwrap();
        assert_eq!(
            super::target(&local, &Activation::Invite("abc".into())).as_deref(),
            Some("http://127.0.0.1:8080/?invite=abc")
        );
    }

    #[test]
    fn a_link_can_never_leave_the_server_origin() {
        let server = parse("https://chat.example.com").unwrap();
        // Not producible by `Activation::parse`, but the mapping must hold anyway.
        let hostile = Activation::Invite("x&channel=1#/../@evil.test/?a=b c".into());
        let url = url::Url::parse(&target(&server, &hostile).unwrap()).unwrap();
        assert_eq!(url.origin(), server.url_origin());
        assert_eq!(url.path(), "/");
        assert_eq!(url.fragment(), None);
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            [("invite".into(), "x&channel=1#/../@evil.test/?a=b c".into())]
        );
    }

    #[test]
    fn command_lines_are_read_like_the_previous_client_did() {
        assert_eq!(args(&[]).activation, Activation::Open);
        assert!(!args(&[]).background);
        assert!(args(&["--background"]).background);
        assert_eq!(
            args(&["sticktalk://invite/abc"]).activation,
            Activation::Invite("abc".into())
        );
        assert_eq!(
            args(&["--background", "sticktalk://message/1/2"]).activation,
            Activation::Message {
                channel_id: 1,
                message_id: 2
            }
        );
        // Malformed or hostile links only open the app.
        for link in [
            "sticktalk://invite/a%22b",
            "sticktalk://open?cmd=quit",
            "sticktalk://evil.test/x",
            "sticktalk:",
        ] {
            assert_eq!(args(&[link]).activation, Activation::Open, "{link}");
        }
        assert_eq!(args(&["https://evil.test/"]).activation, Activation::Open);
    }

    #[test]
    fn smoke_flags_take_their_values() {
        let launch = args(&[
            "--smoke",
            "http://127.0.0.1:8080/",
            "--smoke-eval",
            "document.title",
            "--smoke-wait",
            "250",
            "--smoke-shot",
            "out.png",
        ]);
        assert_eq!(launch.smoke.as_deref(), Some("http://127.0.0.1:8080/"));
        assert_eq!(launch.smoke_eval.as_deref(), Some("document.title"));
        assert_eq!(launch.smoke_wait, Some(250));
        assert_eq!(launch.smoke_shot, Some("out.png".into()));
        assert_eq!(args(&["--smoke"]).smoke, None);
    }
}
