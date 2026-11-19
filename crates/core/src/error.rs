use std::fmt;

use rusqlite::ErrorCode;
use serde::{Serialize, Serializer};

pub const REFUSED_HERE_FOR: i64 = 30 * 86_400_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    NetOffline,
    NetDns,
    NetConnect,
    NetTls,
    NetTimeout,
    NetBlocked,
    NetRefused,
    NetLimited,
    NetServer,
    YtParse,
    YtUnavailable,
    YtdlpMissing,
    YtdlpFailed,
    LibraryLocked,
    LibraryNewer,
    LibraryFailed,
    AudioFailed,
    ApiArgs,
    ApiMethod,
    ApiNotFound,
    ApiWindowOnly,
    SourceUnsupported,
    Internal,
    PlayFailed,
    AutoplayFailed,
    FxReset,
    AudioRestart,
    LibraryUnsaved,
    DownloadDone,
    DownloadFailed,
    CoverUnreadable,
    ImportBlocked,
    ImportUnreadable,
    ImportUnsupported,
    ImportColumns,
    ImportEmpty,
    ShellExtension,
    ImportFile,
    ImportCancelled,
    ProxyInvalid,
    DownloadNoTool,
    YtdlpFetching,
    YtdlpChecksum,
    ShellBlur,
    ShellBlurSkipped,
    ShellBlurUndo,
    YtdlpReady,
    YtdlpCurrent,
    YtdlpFetchFailed,
    ProxyAuth,
    ProxyFailed,
    ProxyDown,
    ProxyTimeout,
    ProxyBlocked,
    LibraryCorrupt,
    UpdateNoRelease,
    UpdateChecksum,
    UpdateBroken,
    UpdatePlace,
    UpdateSignature,
    DownloadNoSpace,
    DownloadFolderInvalid,
    YtAgeRestricted,
    YtRemoved,
    YtRegion,
    NetYoutubeBlocked,
    DownloadListSetAside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryAfter(pub u64);

impl fmt::Display for RetryAfter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "retry after {} s", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Told;

impl fmt::Display for Told {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("already shown to the person")
    }
}

pub const ALL: [(Code, &str); 67] = [
    (Code::NetOffline, "the network is unreachable, or the request could not be sent"),
    (Code::NetDns, "the host name could not be resolved: no network, or DNS failed or is blocked"),
    (Code::NetConnect, "the host refused or reset the connection"),
    (Code::NetTls, "the TLS handshake failed or the certificate was rejected"),
    (Code::NetTimeout, "the host did not answer in time, or the stream stalled"),
    (Code::NetBlocked, "the host answered 403 or 451: requests from this network are refused"),
    (Code::NetRefused, "the host answered with a 4xx status other than 403, 429 and 451"),
    (Code::NetLimited, "the host answered 429, too many requests"),
    (Code::NetServer, "the host answered with a 5xx status"),
    (Code::YtParse, "a YouTube answer is not JSON or lacks what the parser needs"),
    (Code::YtUnavailable, "YouTube will not play this track (playability status) and no fallback could"),
    (Code::YtdlpMissing, "yt-dlp cannot be started"),
    (Code::YtdlpFailed, "yt-dlp ran and failed"),
    (Code::LibraryLocked, "SQLite reports the library busy or locked by another process"),
    (Code::LibraryNewer, "the library has a newer schema than this build; it is left untouched"),
    (Code::LibraryFailed, "any other library or migration failure"),
    (Code::AudioFailed, "mpv failed to start, exited or refused a command"),
    (Code::ApiArgs, "a missing or invalid argument, an unknown setting or a value of the wrong kind"),
    (Code::ApiMethod, "an unknown API method or route"),
    (Code::ApiNotFound, "the queue item, playlist or report asked for does not exist"),
    (Code::ApiWindowOnly, "only the window itself may ask for this; it opens a dialog in front of the person"),
    (Code::SourceUnsupported, "the source cannot do this, or the track belongs to another source"),
    (Code::Internal, "anything not classified above"),
    (Code::PlayFailed, "toast: the current track cannot be played; `cause` says why"),
    (Code::AutoplayFailed, "toast: radio top-up failed at the end of the queue; `cause` says why"),
    (Code::FxReset, "toast: the saved audio effects were rejected by mpv and reset"),
    (Code::AudioRestart, "toast: mpv stopped, it restarts on the next play"),
    (Code::LibraryUnsaved, "toast: the library cannot be opened, the app runs on memory; `cause` says why"),
    (Code::DownloadDone, "toast: `arg` was downloaded"),
    (Code::DownloadFailed, "toast: the download of `arg` failed; `cause` says why"),
    (Code::CoverUnreadable, "the image chosen as a playlist cover is not a PNG, JPEG or WebP file of at most 20 MB, or cannot be read"),
    (Code::ImportBlocked, "the service refused to show the playlist: 401, 403, 404 or 451, a private playlist, a login wall or a captcha"),
    (Code::ImportUnreadable, "the playlist page answered but holds no track list Snellar can read"),
    (Code::ImportUnsupported, "the link is not a Spotify, Deezer, Apple Music, Yandex Music or SoundCloud playlist or album"),
    (Code::ImportColumns, "the CSV file has no column with track names"),
    (Code::ImportEmpty, "the playlist or file has no tracks"),
    (Code::ShellExtension, "GNOME Shell could not install the extension, or does not offer to"),
    (Code::ImportFile, "the file cannot be read as UTF-8 text of at most 5 MB"),
    (Code::ImportCancelled, "the import was cancelled, or replaced by a newer one"),
    (Code::ProxyInvalid, "the proxy setting is not an http:// or https:// address with a host"),
    (Code::DownloadNoTool, "a download needs yt-dlp, it is not there and could not be fetched"),
    (Code::YtdlpFetching, "yt-dlp is being fetched, the percentage is in `arg`"),
    (Code::YtdlpReady, "yt-dlp was installed or updated"),
    (Code::YtdlpCurrent, "yt-dlp was asked to update and is the latest release"),
    (Code::YtdlpFetchFailed, "the button could not fetch yt-dlp, the cause says why"),
    (Code::ShellBlur, "GNOME Shell could not install or start Blur my Shell, or its settings did not take"),
    (Code::ShellBlurSkipped, "Blur my Shell was not installed, usually because the person said no in GNOME's dialog; nothing changed"),
    (Code::ShellBlurUndo, "the Blur my Shell settings could not be put back"),
    (Code::YtdlpChecksum, "the downloaded yt-dlp does not match the checksum its release publishes, nothing was installed"),
    (Code::ProxyAuth, "the proxy answered 407: it wants a login"),
    (Code::ProxyFailed, "the proxy answered with an error instead of opening the connection, such as 502"),
    (Code::ProxyDown, "a proxy is in use and it cannot be reached, or it cut the connection"),
    (Code::ProxyTimeout, "a proxy is in use and no answer came through it in time"),
    (Code::ProxyBlocked, "a proxy is in use and the host answered 403 or 451 through it"),
    (Code::LibraryCorrupt, "the library file is not a readable database; it is left untouched"),
    (Code::UpdateNoRelease, "the latest release does not name a version Snellar can read; nothing was fetched"),
    (Code::UpdateChecksum, "the downloaded Snellar does not match the checksum of its release, the installed one stays"),
    (Code::UpdateBroken, "the downloaded Snellar does not unpack, the installed one stays"),
    (Code::UpdatePlace, "the new Snellar could not be written next to the installed one, the installed one stays"),
    (
        Code::UpdateSignature,
        "the release's checksum file is not signed by the key built into this Snellar, or the signature is missing; nothing was fetched, the installed one stays",
    ),
    (Code::DownloadNoSpace, "the disk of the download folder has less than 1 GiB free; no download starts until there is room"),
    (Code::DownloadFolderInvalid, "the download folder given is relative or climbs out with ..; it must start with / or ~/"),
    (Code::YtAgeRestricted, "YouTube plays this track only to a signed-in adult (age restriction); retrying cannot help"),
    (Code::YtRemoved, "YouTube says the track was removed, is private or does not exist; retrying cannot help"),
    (Code::YtRegion, "YouTube does not offer the track in this country; retrying cannot help until the route changes"),
    (
        Code::NetYoutubeBlocked,
        "YouTube cannot be reached over a direct connection while other hosts answer: the network or the country blocks it; a proxy or a VPN may help",
    ),
    (
        Code::DownloadListSetAside,
        "toast: a saved download list this build cannot read was kept as `<key>.broken` (or `.broken.2`, …) and started again empty; downloaded files stay",
    ),
];

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::NetOffline => "net.offline",
            Code::NetDns => "net.dns",
            Code::NetConnect => "net.connect",
            Code::NetTls => "net.tls",
            Code::NetTimeout => "net.timeout",
            Code::NetBlocked => "net.blocked",
            Code::NetRefused => "net.refused",
            Code::NetLimited => "net.limited",
            Code::NetServer => "net.server",
            Code::YtParse => "yt.parse",
            Code::YtUnavailable => "yt.unavailable",
            Code::YtdlpMissing => "ytdlp.missing",
            Code::YtdlpFailed => "ytdlp.failed",
            Code::LibraryLocked => "library.locked",
            Code::LibraryNewer => "library.newer",
            Code::LibraryFailed => "library.failed",
            Code::AudioFailed => "audio.failed",
            Code::ApiArgs => "api.args",
            Code::ApiMethod => "api.method",
            Code::ApiNotFound => "api.not_found",
            Code::ApiWindowOnly => "api.window_only",
            Code::SourceUnsupported => "source.unsupported",
            Code::Internal => "internal",
            Code::PlayFailed => "play.failed",
            Code::AutoplayFailed => "autoplay.failed",
            Code::FxReset => "fx.reset",
            Code::AudioRestart => "audio.restart",
            Code::LibraryUnsaved => "library.unsaved",
            Code::DownloadDone => "download.done",
            Code::DownloadFailed => "download.failed",
            Code::CoverUnreadable => "cover.unreadable",
            Code::ImportBlocked => "import.blocked",
            Code::ImportUnreadable => "import.unreadable",
            Code::ImportUnsupported => "import.unsupported",
            Code::ImportColumns => "import.columns",
            Code::ImportEmpty => "import.empty",
            Code::ShellExtension => "shell.extension",
            Code::ImportFile => "import.file",
            Code::ImportCancelled => "import.cancelled",
            Code::ProxyInvalid => "proxy.invalid",
            Code::DownloadNoTool => "download.no_tool",
            Code::YtdlpFetching => "ytdlp.fetching",
            Code::YtdlpReady => "ytdlp.ready",
            Code::YtdlpCurrent => "ytdlp.current",
            Code::YtdlpFetchFailed => "ytdlp.fetch_failed",
            Code::YtdlpChecksum => "ytdlp.checksum",
            Code::ShellBlur => "shell.blur",
            Code::ShellBlurSkipped => "shell.blur_skipped",
            Code::ShellBlurUndo => "shell.blur_undo",
            Code::ProxyAuth => "proxy.auth",
            Code::ProxyFailed => "proxy.failed",
            Code::ProxyDown => "proxy.down",
            Code::ProxyTimeout => "proxy.timeout",
            Code::ProxyBlocked => "proxy.blocked",
            Code::LibraryCorrupt => "library.corrupt",
            Code::UpdateNoRelease => "update.no_release",
            Code::UpdateChecksum => "update.checksum",
            Code::UpdateBroken => "update.broken",
            Code::UpdatePlace => "update.place",
            Code::UpdateSignature => "update.signature",
            Code::DownloadNoSpace => "download.no_space",
            Code::DownloadFolderInvalid => "download.folder_invalid",
            Code::YtAgeRestricted => "yt.age_restricted",
            Code::YtRemoved => "yt.removed",
            Code::YtRegion => "yt.region",
            Code::NetYoutubeBlocked => "net.youtube_blocked",
            Code::DownloadListSetAside => "download.list_set_aside",
        }
    }

    pub fn sqlite(e: &rusqlite::Error) -> Code {
        match e.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Code::LibraryLocked,
            Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => Code::LibraryCorrupt,
            _ => Code::LibraryFailed,
        }
    }

    pub fn network(self) -> bool {
        self.as_str().starts_with("net.") || self.as_str().starts_with("proxy.")
    }

    pub fn nothing_gets_through(self) -> bool {
        matches!(
            self,
            Code::NetOffline
                | Code::NetDns
                | Code::NetConnect
                | Code::NetTls
                | Code::NetYoutubeBlocked
                | Code::ProxyDown
                | Code::ProxyAuth
                | Code::ProxyFailed
        )
    }

    pub fn refusal_lasts(self) -> Option<i64> {
        match self {
            Code::YtAgeRestricted | Code::YtRemoved => Some(i64::MAX),
            Code::YtRegion => Some(REFUSED_HERE_FOR),
            _ => None,
        }
    }

    pub fn refused_for_good(self) -> bool {
        self.refusal_lasts().is_some()
    }

    pub fn passes_by_itself(self) -> bool {
        matches!(self, Code::NetLimited | Code::NetServer | Code::NetTimeout | Code::ProxyTimeout | Code::ProxyFailed)
    }

    pub fn retry_after(e: &anyhow::Error) -> Option<u64> {
        e.downcast_ref::<RetryAfter>().map(|r| r.0)
    }

    pub fn err(self, detail: impl Into<String>) -> anyhow::Error {
        anyhow::Error::msg(detail.into()).context(self)
    }

    pub fn of(e: &anyhow::Error) -> Code {
        if let Some(code) = e.downcast_ref::<Code>() {
            return *code;
        }
        for cause in e.chain() {
            if let Some(e) = cause.downcast_ref::<rusqlite::Error>() {
                return Code::sqlite(e);
            }
        }
        Code::Internal
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Code {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
