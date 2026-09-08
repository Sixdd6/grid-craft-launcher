//! URL-keyed cache shared by the icon and description-image caches.
//!
//! Neither icons nor description images have a published hash, so they cannot use the
//! sha1-addressed object store the rest of [`crate::download`] uses. The cache key is the
//! URL instead: a file lands at `<dir>/<sha1(url)>.<ext>`, where `<ext>` is a label taken
//! from the URL's path and not a promise about the bytes (the decoder sniffs the real
//! format). A file already at that path is returned without a request.
//!
//! What the two callers differ in is a [`Policy`]: which hosts are allowed, how large a
//! body may be, whether private hosts are refused, and which cache directory the file
//! lands in. Everything else — the `https://` rule, the redirect guard, the staging file,
//! the streaming cap, and the one request per URL at a time — lives here once.
//!
//! Every rule is applied to the URL the caller passed **and to every redirect hop**: the
//! client this cache streams with is built by [`HttpClient::with_redirect_guard`], so a
//! host that answers 302 cannot move the transfer to plain HTTP, to another host, or to
//! `localhost`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use url::{Host, Url};

use super::{Error, remove_quietly, store_object};
use crate::http::HttpClient;
use crate::paths::{Root, safe_join};

/// Extension used when the URL's path names none.
const DEFAULT_EXT: &str = "img";

/// Host suffixes a [`Policy::refuse_private`] cache never fetches from.
const PRIVATE_SUFFIXES: [&str; 3] = [".localhost", ".internal", ".local"];

/// What one caller of [`MediaCache`] allows.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Hosts a file may be fetched from. `None` means any host, which is what a
    /// description image gets; `Some(list)` is the icon cache's CDN allowlist.
    pub allowed_hosts: Option<Vec<String>>,
    /// Largest body accepted, in bytes. Enforced while the body streams, so nothing
    /// larger is ever written.
    pub max_bytes: u64,
    /// Whether a host that names the machine itself or a private network is refused: an
    /// IP literal, `localhost`, or a `.localhost`, `.internal`, or `.local` name. A host
    /// in `extra_hosts` is exempt, which is what lets a test's wiremock server on
    /// `127.0.0.1` answer.
    pub refuse_private: bool,
    /// The cache directory the file lands in, as a [`Root`] accessor.
    pub dir: fn(&Root) -> PathBuf,
}

/// Downloads URL-keyed files into one cache directory, at most one request per URL at a time.
///
/// It is `Send + Sync`, so any thread may ask it for a file.
#[derive(Debug)]
pub struct MediaCache {
    /// What this cache allows. It is fixed for the cache's life, so the redirect guard
    /// and the up-front check can never disagree.
    policy: Policy,
    /// One lock per URL that is being fetched right now. An entry is removed once the last
    /// caller waiting on it is done, so a long session does not grow the map forever.
    flight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The redirect-guarded client and the `extra_hosts` it was built for. Building one
    /// costs a TLS setup, so it is kept until a caller passes a different test seam.
    client: Mutex<Option<(Vec<String>, HttpClient)>>,
}

impl MediaCache {
    /// An empty cache that allows what `policy` allows.
    #[must_use]
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            flight: Mutex::new(HashMap::new()),
            client: Mutex::new(None),
        }
    }

    /// Returns the cached path for `url` under the policy's directory, downloading it
    /// first when it is not there.
    ///
    /// The URL is parsed with the `url` crate, and its scheme and host must pass the
    /// policy before any request; so must every redirect hop, which is checked again by
    /// the client's redirect guard. Either refusal is [`Error::DisallowedHost`].
    /// `extra_hosts` adds to the allowlist for this call and lets that host be plain HTTP
    /// and private, which is the test seam; pass an empty slice outside a test. Two
    /// concurrent calls for one URL make one request: the second waits on the first and
    /// then finds the file.
    pub async fn fetch(
        &self,
        http: &HttpClient,
        root: &Root,
        url: &str,
        extra_hosts: &[String],
    ) -> Result<PathBuf, Error> {
        let parsed = Url::parse(url).ok();
        let host = parsed
            .as_ref()
            .and_then(|parsed| parsed.host_str())
            .unwrap_or_default()
            .to_string();
        let refused = || Error::DisallowedHost {
            url: url.to_string(),
            host: host.clone(),
        };
        let allowed = parsed
            .as_ref()
            .is_some_and(|parsed| url_allowed(parsed, extra_hosts, &self.policy));
        if !allowed {
            return Err(refused());
        }

        let dest = safe_join(&(self.policy.dir)(root), &cache_file_name(url))?;
        if is_file(&dest).await {
            return Ok(dest);
        }
        let client = self.guarded_client(http, extra_hosts)?;

        let lock = self.lock_for(url);
        let result = {
            let _guard = lock.lock().await;
            if is_file(&dest).await {
                Ok(dest.clone())
            } else {
                download(&client, root, url, &dest, self.policy.max_bytes).await
            }
        };
        self.release(url);
        result
    }

    /// Whether a fetch of `url` is in flight. Test helper.
    #[cfg(test)]
    pub(crate) fn in_flight(&self, url: &str) -> bool {
        self.flight
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .contains_key(url)
    }

    /// The client every hop of a media transfer is checked by, built on first use for
    /// these `extra_hosts` and kept while a caller keeps passing the same ones.
    fn guarded_client(&self, http: &HttpClient, extra: &[String]) -> Result<HttpClient, Error> {
        let mut slot = self.client.lock().unwrap_or_else(|err| err.into_inner());
        if let Some((hosts, client)) = slot.as_ref()
            && hosts.as_slice() == extra
        {
            return Ok(client.clone());
        }
        let policy = self.policy.clone();
        let extra_hosts = extra.to_vec();
        let guard: Arc<dyn Fn(&Url) -> bool + Send + Sync> =
            Arc::new(move |url: &Url| url_allowed(url, &extra_hosts, &policy));
        let client = http.clone().with_redirect_guard(guard)?;
        *slot = Some((extra.to_vec(), client.clone()));
        Ok(client)
    }

    /// The per-URL lock, created on first use.
    fn lock_for(&self, url: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.flight.lock().unwrap_or_else(|err| err.into_inner());
        Arc::clone(map.entry(url.to_string()).or_default())
    }

    /// Drops the per-URL lock once no other caller holds it.
    fn release(&self, url: &str) {
        let mut map = self.flight.lock().unwrap_or_else(|err| err.into_inner());
        // Two strong counts: the map's and the caller's, which is about to go out of scope.
        if map
            .get(url)
            .is_some_and(|lock| Arc::strong_count(lock) <= 2)
        {
            map.remove(url);
        }
    }
}

/// Streams one file into a staging file, under a cap, and moves it to `dest`.
///
/// The cap is applied while the body streams, so a server that keeps sending cannot fill
/// the disk before anyone looks at the size. A refused redirect arrives here as a redirect
/// error and becomes [`Error::DisallowedHost`], the same answer the up-front check gives.
async fn download(
    http: &HttpClient,
    root: &Root,
    url: &str,
    dest: &Path,
    max_bytes: u64,
) -> Result<PathBuf, Error> {
    let part = root
        .objects_dir()
        .join("tmp")
        .join(format!("{}.part", uuid::Uuid::new_v4()));
    match http
        .stream_to_file(url, &part, None, Some(max_bytes), &mut |_| {})
        .await
    {
        Ok(_) => {}
        Err(err) => {
            remove_quietly(&part).await;
            return Err(map_error(url, err));
        }
    };
    store_object(&part, dest).await?;
    Ok(dest.to_path_buf())
}

/// Turns a streaming failure into this module's error.
fn map_error(url: &str, err: crate::http::Error) -> Error {
    if err.is_redirect() {
        let host = Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_default();
        return Error::DisallowedHost {
            url: url.to_string(),
            host,
        };
    }
    match err {
        crate::http::Error::TooLarge { url, size, max } => Error::TooLarge { url, size, max },
        other => Error::Http(other),
    }
}

/// True when `path` is a file that exists.
async fn is_file(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|meta| meta.is_file())
}

/// The cache file name for `url`: its sha1 in hex, then the extension label.
#[must_use]
pub fn cache_file_name(url: &str) -> String {
    format!(
        "{}.{}",
        super::hash::sha1_hex(url.as_bytes()),
        extension_of(url)
    )
}

/// The extension label for a URL, or [`DEFAULT_EXT`] when it carries none worth keeping.
///
/// Only a short all-alphanumeric suffix of the last path segment counts, so a query string
/// or an odd path cannot steer the file name.
fn extension_of(url: &str) -> String {
    let path = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let last = path.rsplit('/').next().unwrap_or_default();
    let ext = last.rsplit_once('.').map_or("", |(_, ext)| ext);
    if !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext.to_ascii_lowercase()
    } else {
        DEFAULT_EXT.to_string()
    }
}

/// Whether a URL may be fetched under `policy`: its scheme, its host, and, when the policy
/// refuses private hosts, whether the host names this machine or a private network.
///
/// This is the one rule the up-front check and the redirect guard both run, so a hop can
/// never reach somewhere the first request could not have.
pub(crate) fn url_allowed(url: &Url, extra: &[String], policy: &Policy) -> bool {
    let Some(host) = url.host() else {
        return false;
    };
    let host_str = url.host_str().unwrap_or_default();
    let named = is_named(host_str, extra);
    if !scheme_allowed(url, named) {
        return false;
    }
    if policy.refuse_private && !named && is_private(&host) {
        return false;
    }
    host_allowed(host_str, extra, policy)
}

/// Whether a URL's scheme is allowed.
///
/// It must be `https`. The one exception is a host a caller named in `extra_hosts`, which
/// is the test seam ([`crate::Launcher::with_icon_hosts`] and
/// [`crate::Launcher::with_image_hosts`], both empty in a shipped launcher): a wiremock
/// server speaks plain HTTP.
fn scheme_allowed(url: &Url, named: bool) -> bool {
    url.scheme() == "https" || named
}

/// Whether `host` is one the caller named in `extra_hosts`.
///
/// An IPv6 host is compared without its brackets too, so `::1` matches `[::1]`.
fn is_named(host: &str, extra: &[String]) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    extra.iter().any(|allowed| {
        let allowed = allowed.trim();
        allowed.eq_ignore_ascii_case(host) || allowed.eq_ignore_ascii_case(bare)
    })
}

/// Whether a host names this machine or a private network rather than a public CDN.
///
/// Every IP literal counts, v4 and v6 alike: a description that names one is either a
/// mistake or an attempt to make the launcher reach something on the user's own network.
fn is_private(host: &Host<&str>) -> bool {
    match host {
        Host::Ipv4(_) | Host::Ipv6(_) => true,
        Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            name == "localhost" || PRIVATE_SUFFIXES.iter().any(|end| name.ends_with(end))
        }
    }
}

/// Whether a file may be fetched from `host` under `policy`.
///
/// A policy with no host list allows every host, so only the scheme and private-host rules
/// apply there.
pub(crate) fn host_allowed(host: &str, extra: &[String], policy: &Policy) -> bool {
    let Some(allowed) = policy.allowed_hosts.as_ref() else {
        return true;
    };
    allowed
        .iter()
        .chain(extra.iter())
        .any(|allowed| allowed.trim().eq_ignore_ascii_case(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_policy() -> Policy {
        Policy {
            allowed_hosts: None,
            max_bytes: 1,
            refuse_private: true,
            dir: Root::images_dir,
        }
    }

    fn cdn_policy() -> Policy {
        Policy {
            allowed_hosts: Some(vec!["cdn.example".to_string()]),
            max_bytes: 1,
            refuse_private: true,
            dir: Root::icons_dir,
        }
    }

    fn allowed(url: &str, extra: &[String], policy: &Policy) -> bool {
        Url::parse(url).is_ok_and(|parsed| url_allowed(&parsed, extra, policy))
    }

    #[test]
    fn cache_file_name_is_the_url_sha1_plus_its_extension() {
        let url = "https://cdn.modrinth.com/data/AAAA/icon.png";
        let sha1 = super::super::hash::sha1_hex(url.as_bytes());
        assert_eq!(cache_file_name(url), format!("{sha1}.png"));
    }

    #[test]
    fn a_url_without_a_usable_extension_gets_the_default_label() {
        assert!(cache_file_name("https://cdn.modrinth.com/data/AAAA/icon").ends_with(".img"));
        assert!(cache_file_name("https://cdn.modrinth.com/icon.p%20g").ends_with(".img"));
        assert!(cache_file_name("https://cdn.modrinth.com/i.PNG?w=64").ends_with(".png"));
    }

    #[test]
    fn a_policy_without_a_host_list_allows_every_public_host() {
        assert!(allowed(
            "https://anything.example/a.png",
            &[],
            &open_policy()
        ));
        assert!(allowed("https://cdn.example/a.png", &[], &cdn_policy()));
        assert!(!allowed("https://evil.example/a.png", &[], &cdn_policy()));
        assert!(allowed(
            "https://evil.example/a.png",
            &["evil.example".to_string()],
            &cdn_policy()
        ));
    }

    #[test]
    fn only_https_passes_unless_the_host_is_a_named_test_seam() {
        assert!(!allowed("http://cdn.example/a.png", &[], &cdn_policy()));
        assert!(!allowed("ftp://cdn.example/a.png", &[], &cdn_policy()));
        assert!(allowed(
            "http://cdn.example/a.png",
            &["cdn.example".to_string()],
            &cdn_policy()
        ));
        assert!(!allowed("cdn.example/a.png", &[], &cdn_policy()));
    }

    #[test]
    fn a_private_host_is_refused_by_a_policy_that_asks_for_it() {
        for url in [
            "https://127.0.0.1/a.png",
            "https://10.1.2.3/a.png",
            "https://[::1]/a.png",
            "https://[fd00::1]/a.png",
            "https://localhost/a.png",
            "https://build.localhost/a.png",
            "https://files.internal/a.png",
            "https://nas.local/a.png",
            "https://NAS.LOCAL./a.png",
        ] {
            assert!(!allowed(url, &[], &open_policy()), "{url}");
        }
        assert!(allowed("https://images.example/a.png", &[], &open_policy()));
    }

    #[test]
    fn a_named_test_host_is_exempt_from_the_private_rule() {
        let hosts = vec!["127.0.0.1".to_string(), "localhost".to_string()];
        assert!(allowed(
            "http://127.0.0.1:8080/a.png",
            &hosts,
            &open_policy()
        ));
        assert!(allowed(
            "http://localhost:8080/a.png",
            &hosts,
            &open_policy()
        ));
        assert!(allowed(
            "http://[::1]:8080/a.png",
            &["::1".to_string()],
            &open_policy()
        ));
    }

    #[test]
    fn userinfo_does_not_make_a_url_the_cdn_it_names() {
        // The `url` crate reads the authority the way a browser does: the backslash ends
        // it, so the host is `evil.example` and the CDN name is only path text.
        assert!(!allowed(
            "https://evil.example\\@cdn.example/a.png",
            &[],
            &cdn_policy()
        ));
        assert!(!allowed(
            "https://cdn.example@evil.example/a.png",
            &[],
            &cdn_policy()
        ));
        assert!(allowed(
            "https://user:pw@cdn.example:8443/a.png",
            &[],
            &cdn_policy()
        ));
    }
}
