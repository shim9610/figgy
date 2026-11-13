//! One export call by format, and saving it to a file (#265).
//!
//! Each call routes to the existing `export_png`, `export_svg` or
//! `export_pdf` path; nothing renders differently from those.

use crate::radial::{RadialChart, RadialError, RadialRenderer};
use crate::{ExportOptions, ExportSource, FiggyError, Renderer};

/// The file formats an export writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportFormat {
    /// A raster image ([`Renderer::export_png`]).
    Png,
    /// A vector document with the data embedded as images
    /// ([`Renderer::export_svg`]).
    Svg,
    /// A one-page vector document ([`Renderer::export_pdf`]).
    Pdf,
}

impl ExportFormat {
    /// The format a file name's extension names, in any letter case:
    /// `png`, `svg` or `pdf`. `None` for any other extension or none.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Option<Self> {
        let extension = path.as_ref().extension()?.to_str()?;
        [Self::Png, Self::Svg, Self::Pdf]
            .into_iter()
            .find(|format| format.extension().eq_ignore_ascii_case(extension))
    }

    /// The file extension, without the dot.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
        }
    }
}

/// Why a save failed.
#[derive(Debug)]
pub enum SaveError {
    /// The renderer could not export the chart.
    Export(FiggyError),
    /// The radial renderer could not export the chart.
    Radial(RadialError),
    /// The file name's extension names no [`ExportFormat`]; save with an
    /// explicit format instead.
    UnknownFormat(std::path::PathBuf),
    /// Writing the file failed.
    Write {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Export(error) => error.fmt(f),
            Self::Radial(error) => error.fmt(f),
            Self::UnknownFormat(path) => write!(
                f,
                "no export format for the file name {path:?}: use png, svg or pdf, or name the format"
            ),
            Self::Write { path, source } => write!(f, "writing {path:?} failed: {source}"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Export(error) => Some(error),
            Self::Radial(error) => Some(error),
            Self::UnknownFormat(_) => None,
            Self::Write { source, .. } => Some(source),
        }
    }
}

impl From<FiggyError> for SaveError {
    fn from(error: FiggyError) -> Self {
        Self::Export(error)
    }
}

impl From<RadialError> for SaveError {
    fn from(error: RadialError) -> Self {
        Self::Radial(error)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn format_of(path: &std::path::Path) -> Result<ExportFormat, SaveError> {
    ExportFormat::from_path(path).ok_or_else(|| SaveError::UnknownFormat(path.to_path_buf()))
}

#[cfg(not(target_arch = "wasm32"))]
fn write(path: &std::path::Path, bytes: &[u8]) -> Result<(), SaveError> {
    std::fs::write(path, bytes).map_err(|source| SaveError::Write {
        path: path.to_path_buf(),
        source,
    })
}

impl Renderer {
    /// `source` exported as `format`, in memory: [`Self::export_png`],
    /// [`Self::export_svg`] (UTF-8 bytes) or [`Self::export_pdf`].
    pub async fn export_bytes(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
        format: ExportFormat,
    ) -> crate::Result<Vec<u8>> {
        match format {
            ExportFormat::Png => self.export_png(source, options).await,
            ExportFormat::Svg => self
                .export_svg(source, options)
                .await
                .map(String::into_bytes),
            ExportFormat::Pdf => self.export_pdf(source, options).await,
        }
    }

    /// [`Self::export_bytes`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_bytes_blocking(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
        format: ExportFormat,
    ) -> crate::Result<Vec<u8>> {
        match format {
            ExportFormat::Png => self.export_png_blocking(source, options),
            ExportFormat::Svg => self
                .export_svg_blocking(source, options)
                .map(String::into_bytes),
            ExportFormat::Pdf => self.export_pdf_blocking(source, options),
        }
    }

    /// Export `source` and write it to `path`, in the format its extension
    /// names ([`ExportFormat::from_path`]).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_blocking(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), SaveError> {
        let path = path.as_ref();
        let format = format_of(path)?;
        self.save_as_blocking(source, options, path, format)
    }

    /// Export `source` as `format` and write it to `path`, whatever its
    /// extension.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_as_blocking(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
        path: impl AsRef<std::path::Path>,
        format: ExportFormat,
    ) -> Result<(), SaveError> {
        let bytes = self.export_bytes_blocking(source, options, format)?;
        write(path.as_ref(), &bytes)
    }
}

impl crate::RasterImage {
    /// Encode the image as PNG ([`crate::encode_png`]) and write it to
    /// `path`, for a host that checks the pixels before saving them.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_png(&self, path: impl AsRef<std::path::Path>) -> Result<(), SaveError> {
        let bytes = crate::encode_png(self)?;
        write(path.as_ref(), &bytes)
    }
}

impl RadialRenderer {
    /// `chart` at `size` exported as `format`, in memory: the radial
    /// counterpart of [`Renderer::export_bytes`].
    pub async fn export_bytes(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: ExportOptions,
        format: ExportFormat,
    ) -> Result<Vec<u8>, RadialError> {
        match format {
            ExportFormat::Png => self.export_png(chart, size, options).await,
            ExportFormat::Svg => self
                .export_svg(chart, size, options)
                .await
                .map(String::into_bytes),
            ExportFormat::Pdf => self.export_pdf(chart, size, options).await,
        }
    }

    /// [`Self::export_bytes`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_bytes_blocking(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: ExportOptions,
        format: ExportFormat,
    ) -> Result<Vec<u8>, RadialError> {
        match format {
            ExportFormat::Png => self.export_png_blocking(chart, size, options),
            ExportFormat::Svg => self
                .export_svg_blocking(chart, size, options)
                .map(String::into_bytes),
            ExportFormat::Pdf => self.export_pdf_blocking(chart, size, options),
        }
    }

    /// Export `chart` and write it to `path`, in the format its extension
    /// names.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_blocking(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        options: ExportOptions,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), SaveError> {
        let path = path.as_ref();
        let format = format_of(path)?;
        let bytes = self.export_bytes_blocking(chart, size, options, format)?;
        write(path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::ExportFormat;

    #[test]
    fn the_format_comes_from_the_extension_in_any_case() {
        assert_eq!(ExportFormat::from_path("a/b.png"), Some(ExportFormat::Png));
        assert_eq!(
            ExportFormat::from_path("chart.SVG"),
            Some(ExportFormat::Svg)
        );
        assert_eq!(ExportFormat::from_path("x.Pdf"), Some(ExportFormat::Pdf));
        assert_eq!(ExportFormat::from_path("x.jpeg"), None);
        assert_eq!(ExportFormat::from_path("noextension"), None);
        for format in [ExportFormat::Png, ExportFormat::Svg, ExportFormat::Pdf] {
            let name = format!("f.{}", format.extension());
            assert_eq!(ExportFormat::from_path(name), Some(format));
        }
    }
}
