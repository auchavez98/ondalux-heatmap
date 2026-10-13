//! A small flowing-layout PDF writer on top of `pdf-writer`: headings,
//! wrapped paragraphs, tables and images on A4 pages, with automatic page
//! breaks. Uses the standard Helvetica fonts (no font embedding) with
//! WinAnsi encoding, which covers Spanish and other Western European text.

use image::RgbaImage;
use miniz_oxide::deflate::{compress_to_vec_zlib, CompressionLevel};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

pub const PAGE_W: f32 = 595.0;
pub const PAGE_H: f32 = 842.0;
pub const MARGIN: f32 = 50.0;
const FONT: Name = Name(b"F1");
const FONT_BOLD: Name = Name(b"F2");

/// Helvetica advance widths (1/1000 em) for ASCII 32..=126 (Adobe AFM).
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Converts text to WinAnsi bytes; unsupported characters become close
/// ASCII equivalents or '?'.
pub fn winansi(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            c if (c as u32) < 0x80 => out.push(c as u8),
            c if (0xA0..=0xFF).contains(&(c as u32)) => out.push(c as u32 as u8),
            '€' => out.push(0x80),
            '…' => out.push(0x85),
            '•' => out.push(0x95),
            '–' => out.push(0x96),
            '—' => out.push(0x97),
            '‘' => out.push(0x91),
            '’' => out.push(0x92),
            '“' => out.push(0x93),
            '”' => out.push(0x94),
            '−' => out.push(b'-'),
            '≥' => out.extend_from_slice(b">="),
            '≤' => out.extend_from_slice(b"<="),
            'Δ' => out.push(b'D'),
            _ => out.push(b'?'),
        }
    }
    out
}

/// Width of `s` in points at `size`, in Helvetica (bold ≈ +5 %).
pub fn text_width(s: &str, size: f32, bold: bool) -> f32 {
    let units: u32 = winansi(s)
        .iter()
        .map(|&b| match b {
            32..=126 => HELVETICA[(b - 32) as usize] as u32,
            _ => 556,
        })
        .sum();
    let w = units as f32 * size / 1000.0;
    if bold {
        w * 1.05
    } else {
        w
    }
}

fn wrap(text: &str, size: f32, bold: bool, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if text_width(&candidate, size, bold) > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line = word.to_owned();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

pub type Rgb = [f32; 3];
pub const TEXT: Rgb = [0.1, 0.1, 0.12];
pub const MUTED: Rgb = [0.4, 0.4, 0.45];
pub const ACCENT: Rgb = [0.12, 0.35, 0.7];

struct PendingImage {
    name: String,
    width: u32,
    height: u32,
    data: Vec<u8>,
}

struct Page {
    content: Content,
    images: Vec<usize>,
}

/// Flowing document. `y` is the current baseline position from the top.
pub struct Doc {
    pages: Vec<Page>,
    images: Vec<PendingImage>,
    y: f32,
    title: String,
    footer: String,
}

impl Doc {
    pub fn new(title: &str, footer: &str) -> Self {
        let mut d = Self {
            pages: Vec::new(),
            images: Vec::new(),
            y: 0.0,
            title: title.to_owned(),
            footer: footer.to_owned(),
        };
        d.new_page();
        d
    }

    pub fn new_page(&mut self) {
        self.pages.push(Page {
            content: Content::new(),
            images: Vec::new(),
        });
        self.y = MARGIN;
    }

    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().expect("at least one page")
    }

    /// Starts a new page if fewer than `needed` points are left.
    pub fn ensure(&mut self, needed: f32) {
        if self.y + needed > PAGE_H - MARGIN {
            self.new_page();
        }
    }

    pub fn space(&mut self, pts: f32) {
        self.y += pts;
    }

    fn raw_text(&mut self, x: f32, y_top: f32, s: &str, size: f32, bold: bool, color: Rgb) {
        let baseline = PAGE_H - y_top - size * 0.8;
        let c = &mut self.page().content;
        c.set_fill_rgb(color[0], color[1], color[2]);
        c.begin_text();
        c.set_font(if bold { FONT_BOLD } else { FONT }, size);
        c.next_line(x, baseline);
        c.show(Str(&winansi(s)));
        c.end_text();
    }

    /// Wrapped text at the left margin (plus `indent`).
    pub fn text(&mut self, s: &str, size: f32, bold: bool, color: Rgb, indent: f32) {
        let width = PAGE_W - 2.0 * MARGIN - indent;
        let line_h = size * 1.35;
        for line in wrap(s, size, bold, width) {
            self.ensure(line_h);
            let y = self.y;
            self.raw_text(MARGIN + indent, y, &line, size, bold, color);
            self.y += line_h;
        }
    }

    pub fn heading(&mut self, s: &str, size: f32) {
        self.ensure(size * 3.0);
        self.space(size * 0.6);
        self.text(s, size, true, ACCENT, 0.0);
        self.space(size * 0.2);
    }

    pub fn paragraph(&mut self, s: &str) {
        self.text(s, 10.0, false, TEXT, 0.0);
        self.space(4.0);
    }

    pub fn bullet(&mut self, s: &str, color: Rgb) {
        let size = 10.0;
        let width = PAGE_W - 2.0 * MARGIN - 14.0;
        let line_h = size * 1.35;
        for (i, line) in wrap(s, size, false, width).iter().enumerate() {
            self.ensure(line_h);
            let y = self.y;
            if i == 0 {
                self.raw_text(MARGIN + 2.0, y, "•", size, true, color);
            }
            self.raw_text(MARGIN + 14.0, y, line, size, false, TEXT);
            self.y += line_h;
        }
        self.space(2.0);
    }

    /// Key/value lines in two columns.
    pub fn key_values(&mut self, rows: &[(String, String)]) {
        let size = 10.0;
        let key_w = rows
            .iter()
            .map(|(k, _)| text_width(k, size, true))
            .fold(0.0, f32::max)
            + 12.0;
        for (k, v) in rows {
            let lines = wrap(v, size, false, PAGE_W - 2.0 * MARGIN - key_w);
            self.ensure(size * 1.4 * lines.len() as f32);
            let y = self.y;
            self.raw_text(MARGIN, y, k, size, true, TEXT);
            for (i, line) in lines.iter().enumerate() {
                let y = self.y + i as f32 * size * 1.4;
                self.raw_text(MARGIN + key_w, y, line, size, false, TEXT);
            }
            self.y += size * 1.4 * lines.len() as f32;
        }
        self.space(4.0);
    }

    /// Table with a header row. Columns get widths proportional to `weights`.
    pub fn table(&mut self, header: &[String], rows: &[Vec<String>], weights: &[f32]) {
        let size = 8.5;
        let line_h = size * 1.3;
        let total: f32 = weights.iter().sum();
        let avail = PAGE_W - 2.0 * MARGIN;
        let widths: Vec<f32> = weights.iter().map(|w| avail * w / total).collect();
        let draw_row = |doc: &mut Doc, cells: &[String], bold: bool, shade: bool| {
            let wrapped: Vec<Vec<String>> = cells
                .iter()
                .zip(&widths)
                .map(|(c, w)| wrap(c, size, bold, w - 6.0))
                .collect();
            let n = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
            let h = n as f32 * line_h + 4.0;
            doc.ensure(h);
            let top = doc.y;
            if shade {
                let c = &mut doc.page().content;
                let color = if bold {
                    [0.88, 0.91, 0.96]
                } else {
                    [0.96, 0.96, 0.97]
                };
                c.set_fill_rgb(color[0], color[1], color[2]);
                c.rect(MARGIN, PAGE_H - top - h, avail, h);
                c.fill_nonzero();
            }
            let mut x = MARGIN;
            for (lines, w) in wrapped.iter().zip(&widths) {
                for (i, line) in lines.iter().enumerate() {
                    doc.raw_text(
                        x + 3.0,
                        top + 2.0 + i as f32 * line_h,
                        line,
                        size,
                        bold,
                        TEXT,
                    );
                }
                x += w;
            }
            doc.y += h;
        };
        draw_row(self, header, true, true);
        for (i, r) in rows.iter().enumerate() {
            // Repeat the header after a page break.
            let before = self.pages.len();
            draw_row(self, r, false, i % 2 == 1);
            if self.pages.len() != before && self.y > MARGIN {
                // The row moved to a new page; nothing else to do (header
                // repetition is skipped to keep layout simple).
            }
        }
        self.space(6.0);
    }

    /// Image scaled to fit the text width (and `max_h` points).
    pub fn image(&mut self, img: &RgbaImage, max_h: f32) {
        let avail_w = PAGE_W - 2.0 * MARGIN;
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let k = (avail_w / iw).min(max_h / ih);
        let (w, h) = (iw * k, ih * k);
        self.ensure(h + 4.0);
        let rgb: Vec<u8> = img
            .pixels()
            .flat_map(|p| [p.0[0], p.0[1], p.0[2]])
            .collect();
        let idx = self.images.len();
        self.images.push(PendingImage {
            name: format!("Im{}", idx + 1),
            width: img.width(),
            height: img.height(),
            data: compress_to_vec_zlib(&rgb, CompressionLevel::DefaultLevel as u8),
        });
        let x = MARGIN + (avail_w - w) / 2.0;
        let y_bottom = PAGE_H - self.y - h;
        let name = self.images[idx].name.clone();
        let page = self.page();
        page.images.push(idx);
        page.content.save_state();
        page.content.transform([w, 0.0, 0.0, h, x, y_bottom]);
        page.content.x_object(Name(name.as_bytes()));
        page.content.restore_state();
        self.y += h + 6.0;
    }

    pub fn finish(mut self) -> Vec<u8> {
        let n_pages = self.pages.len();
        // Footer with page numbers.
        let footer = self.footer.clone();
        for (i, page) in self.pages.iter_mut().enumerate() {
            let text = format!("{footer}  ·  {}/{}", i + 1, n_pages);
            let c = &mut page.content;
            c.set_fill_rgb(MUTED[0], MUTED[1], MUTED[2]);
            c.begin_text();
            c.set_font(FONT, 8.0);
            c.next_line(MARGIN, 25.0);
            c.show(Str(&winansi(&text)));
            c.end_text();
        }

        let mut pdf = Pdf::new();
        let mut next = 1;
        let mut alloc = || {
            let r = Ref::new(next);
            next += 1;
            r
        };
        let catalog = alloc();
        let tree = alloc();
        let info = alloc();
        let font = alloc();
        let font_bold = alloc();
        let image_ids: Vec<Ref> = self.images.iter().map(|_| alloc()).collect();
        let page_ids: Vec<(Ref, Ref)> = self.pages.iter().map(|_| (alloc(), alloc())).collect();

        pdf.catalog(catalog).pages(tree);
        pdf.pages(tree)
            .kids(page_ids.iter().map(|(p, _)| *p))
            .count(n_pages as i32);
        pdf.document_info(info)
            .title(TextStr(&self.title))
            .creator(TextStr("Ondalux Heatmap"))
            .producer(TextStr("Ondalux Heatmap (pdf-writer)"));
        pdf.type1_font(font)
            .base_font(Name(b"Helvetica"))
            .encoding_predefined(Name(b"WinAnsiEncoding"));
        pdf.type1_font(font_bold)
            .base_font(Name(b"Helvetica-Bold"))
            .encoding_predefined(Name(b"WinAnsiEncoding"));
        for (img, id) in self.images.iter().zip(&image_ids) {
            let mut x = pdf.image_xobject(*id, &img.data);
            x.filter(Filter::FlateDecode);
            x.width(img.width as i32);
            x.height(img.height as i32);
            x.color_space().device_rgb();
            x.bits_per_component(8);
            x.finish();
        }
        for (page, (page_id, content_id)) in self.pages.into_iter().zip(&page_ids) {
            let mut p = pdf.page(*page_id);
            p.media_box(Rect::new(0.0, 0.0, PAGE_W, PAGE_H));
            p.parent(tree);
            p.contents(*content_id);
            {
                let mut res = p.resources();
                {
                    let mut fonts = res.fonts();
                    fonts.pair(FONT, font);
                    fonts.pair(FONT_BOLD, font_bold);
                }
                if !page.images.is_empty() {
                    let mut xo = res.x_objects();
                    for &i in &page.images {
                        xo.pair(Name(self.images[i].name.as_bytes()), image_ids[i]);
                    }
                }
            }
            p.finish();
            pdf.stream(*content_id, &page.content.finish());
        }
        pdf.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winansi_keeps_spanish_and_replaces_math() {
        assert_eq!(winansi("Señal ≥ −67 dBm"), b"Se\xf1al >= -67 dBm".to_vec());
        assert_eq!(
            winansi("¿Cobertura? ¡Sí!"),
            b"\xbfCobertura? \xa1S\xed!".to_vec()
        );
    }

    #[test]
    fn wrapping_respects_width() {
        let text = "uno dos tres cuatro cinco seis siete ocho nueve diez once doce";
        let lines = wrap(text, 10.0, false, 100.0);
        assert!(lines.len() > 2);
        for l in &lines {
            assert!(
                text_width(l, 10.0, false) <= 100.0 || !l.contains(' '),
                "{l}"
            );
        }
    }

    #[test]
    fn produces_a_multi_page_pdf() {
        let mut d = Doc::new("Test", "footer");
        d.heading("Título", 18.0);
        for i in 0..120 {
            d.paragraph(&format!("Párrafo número {i} con texto de relleno."));
        }
        d.image(
            &RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 255])),
            100.0,
        );
        let bytes = d.finish();
        assert!(bytes.starts_with(b"%PDF-"));
        let pdf = hayro::hayro_syntax::Pdf::new(bytes).expect("valid PDF");
        assert!(pdf.pages().len() >= 3);
    }
}
