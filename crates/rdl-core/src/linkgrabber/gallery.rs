//! The gallery-dl side of link checking. A gallery stays **one** link (not
//! one per file): gallery-dl downloads the whole set itself, and its count,
//! size and preview come from `-j`.

use tokio_util::sync::CancellationToken;
use url::Url;

use super::media::{self, Verdict};
use super::{Inner, LinkId};
use crate::gallerydl::{self, Gallery, GalleryDlOptions};
use crate::http::filename;
use crate::manager::DownloadKind;
use crate::proxy::ProxyChoice;
use crate::request::RequestContext;
use crate::tools::ToolName;
use crate::{Error, Result};

/// Ask gallery-dl about `url` and record a gallery it finds. Failures are
/// returned, not recorded, so the caller can pick whose message to show.
pub(super) async fn resolve_link(
    inner: &Inner,
    id: LinkId,
    url: &Url,
    request: &RequestContext,
    default_package: Option<&str>,
) -> Result<Verdict> {
    let Some(tools) = &inner.tools else {
        return Ok(Verdict::NotMedia);
    };
    let exe = match tools.ensure(ToolName::Gallerydl).await {
        Ok(exe) => exe,
        Err(e) => {
            tracing::warn!(error = %e, "gallery-dl unavailable; skipping gallery check");
            return Ok(Verdict::NotMedia);
        }
    };
    let connection = inner.network.tool_connection(url, ProxyChoice::Auto)?;
    let cancel = CancellationToken::new();
    let resolved = match gallerydl::resolve(&exe, url, request, &connection, &cancel).await {
        Ok(r) => r,
        Err(Error::Tool(message)) if gallerydl::is_unsupported(&message) => {
            return Ok(Verdict::NotMedia);
        }
        Err(Error::Tool(message)) => return Ok(Verdict::Failed(message)),
        Err(e) => return Err(e),
    };
    let gallery = resolved.gallery;
    if gallery.files == 0 && gallery.sub_galleries == 0 && !gallery.partial {
        return Ok(Verdict::Failed(match resolved.error {
            Some(e) => e.text(),
            None => "gallery-dl found no files here".into(),
        }));
    }
    if let Some(e) = &resolved.error {
        // Some files were listed before it stopped; the download will retry.
        tracing::info!(link = id, error = %e.text(), "gallery listed with an error");
    }
    record_gallery(inner, id, url, &gallery, default_package).await?;
    Ok(Verdict::Handled)
}

async fn record_gallery(
    inner: &Inner,
    id: LinkId,
    url: &Url,
    gallery: &Gallery,
    default_package: Option<&str>,
) -> Result<()> {
    let options: GalleryDlOptions = media::current_options(inner, id).await?;
    let folder = gallery
        .title
        .clone()
        .or_else(|| filename::from_url(url))
        .map(|t| filename::sanitize(&t));
    // COALESCE keeps a name the user typed meanwhile.
    sqlx::query(
        "UPDATE link_grabber SET kind = ?, availability = 'online', error = NULL,
             filename = COALESCE(filename, ?), size = ?, info_json = ?, handler_opts_json = ?
         WHERE id = ?",
    )
    .bind(DownloadKind::Gallerydl.as_str())
    .bind(folder.as_deref())
    .bind(gallery.size.map(|s| s as i64))
    .bind(serde_json::to_string(gallery)?)
    .bind(serde_json::to_string(&options)?)
    .bind(id)
    .execute(&inner.pool)
    .await?;
    if let (Some(default), Some(title)) = (default_package, &gallery.title) {
        media::rename_default_package(inner, id, default, title).await?;
    }
    tracing::info!(
        link = id,
        files = gallery.files,
        sub_galleries = gallery.sub_galleries,
        "gallery found"
    );
    Ok(())
}
