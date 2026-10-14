//! A small flowing-layout PDF writer on top of `pdf-writer`, with the
//! report's visual style: header with the logo on every page (except the
//! cover), footer with page numbers, section titles, cards, KPI tiles,
//! modern tables and callouts. A4 portrait, standard Helvetica fonts (no
//! embedding) with WinAnsi encoding (Spanish and Western European text).

use image::RgbaImage;
use miniz_oxide::deflate::{compress_to_vec_zlib, CompressionLevel};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

pub const PAGE_W: f32 = 595.0;
pub const PAGE_H: f32 = 842.0;
pub const MARGIN: f32 = 48.0;
/// Content starts below the page header.
pub const CONTENT_TOP: f32 = 82.0;
/// Content ends above the footer.
pub const CONTENT_BOTTOM: f32 = PAGE_H - 58.0;
pub const CONTENT_W: f32 = PAGE_W - 2.0 * MARGIN;
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

pub fn wrap(text: &str, size: f32, bold: bool, width: f32) -> Vec<String> {
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
/// Dark slate for titles.
pub const INK: Rgb = [0.06, 0.09, 0.16];
pub const TEXT: Rgb = [0.18, 0.21, 0.27];
pub const MUTED: Rgb = [0.42, 0.47, 0.55];
pub const LINE: Rgb = [0.88, 0.9, 0.93];
pub const SURFACE: Rgb = [0.965, 0.972, 0.98];
pub const ACCENT: Rgb = [0.145, 0.388, 0.922];
pub const ACCENT_SOFT: Rgb = [0.9, 0.93, 1.0];
pub const OK: Rgb = [0.09, 0.64, 0.29];
pub const WARN: Rgb = [0.85, 0.47, 0.02];
pub const BAD: Rgb = [0.86, 0.15, 0.15];
pub const WHITE: Rgb = [1.0, 1.0, 1.0];

/// A big number with a caption, for KPI rows.
pub struct Kpi {
    pub value: String,
    pub label: String,
    pub color: Rgb,
}

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

/// Flowing document. `y` is the current position from the top of the page.
pub struct Doc {
    pages: Vec<Page>,
    images: Vec<PendingImage>,
    y: f32,
    title: String,
    /// Text on the right of the page header.
    header: String,
    footer: String,
    logo: Option<usize>,
}

impl Doc {
    /// `logo` is drawn in the header of every page but the first (cover).
    pub fn new(title: &str, header: &str, footer: &str, logo: Option<&RgbaImage>) -> Self {
        let mut d = Self {
            pages: Vec::new(),
            images: Vec::new(),
            y: 0.0,
            title: title.to_owned(),
            header: header.to_owned(),
            footer: footer.to_owned(),
            logo: None,
        };
        d.logo = logo.map(|img| d.add_image(img));
        d.new_page();
        d
    }

    pub fn new_page(&mut self) {
        self.pages.push(Page {
            content: Content::new(),
            images: Vec::new(),
        });
        self.y = CONTENT_TOP;
    }

    pub fn y(&self) -> f32 {
        self.y
    }

    pub fn set_y(&mut self, y: f32) {
        self.y = y;
    }

    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().expect("at least one page")
    }

    /// Starts a new page if fewer than `needed` points are left.
    pub fn ensure(&mut self, needed: f32) {
        if self.y + needed > CONTENT_BOTTOM {
            self.new_page();
        }
    }

    pub fn space(&mut self, pts: f32) {
        self.y += pts;
    }

    // ------------------------------------------------------------ primitives

    /// Text with its top at `y_top` (points from the page top).
    pub fn text_at(&mut self, x: f32, y_top: f32, s: &str, size: f32, bold: bool, color: Rgb) {
        let baseline = PAGE_H - y_top - size * 0.8;
        let c = &mut self.page().content;
        c.set_fill_rgb(color[0], color[1], color[2]);
        c.begin_text();
        c.set_font(if bold { FONT_BOLD } else { FONT }, size);
        c.next_line(x, baseline);
        c.show(Str(&winansi(s)));
        c.end_text();
    }

    /// Filled rectangle, `y_top` from the page top.
    pub fn fill_rect(&mut self, x: f32, y_top: f32, w: f32, h: f32, color: Rgb) {
        let c = &mut self.page().content;
        c.set_fill_rgb(color[0], color[1], color[2]);
        c.rect(x, PAGE_H - y_top - h, w, h);
        c.fill_nonzero();
    }

    /// Rounded rectangle, filled and/or stroked.
    #[allow(clippy::too_many_arguments)]
    pub fn round_rect(
        &mut self,
        x: f32,
        y_top: f32,
        w: f32,
        h: f32,
        r: f32,
        fill: Option<Rgb>,
        stroke: Option<Rgb>,
    ) {
        let r = r.min(w / 2.0).min(h / 2.0);
        let (x0, y0, x1, y1) = (x, PAGE_H - y_top - h, x + w, PAGE_H - y_top);
        let k = r * 0.5523;
        let c = &mut self.page().content;
        c.move_to(x0 + r, y0);
        c.line_to(x1 - r, y0);
        c.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
        c.line_to(x1, y1 - r);
        c.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
        c.line_to(x0 + r, y1);
        c.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
        c.line_to(x0, y0 + r);
        c.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
        c.close_path();
        if let Some(f) = fill {
            c.set_fill_rgb(f[0], f[1], f[2]);
        }
        if let Some(s) = stroke {
            c.set_stroke_rgb(s[0], s[1], s[2]);
            c.set_line_width(0.8);
        }
        match (fill, stroke) {
            (Some(_), Some(_)) => {
                c.fill_nonzero_and_stroke();
            }
            (Some(_), None) => {
                c.fill_nonzero();
            }
            (None, Some(_)) => {
                c.stroke();
            }
            (None, None) => {
                c.end_path();
            }
        }
    }

    pub fn hline(&mut self, x: f32, y_top: f32, w: f32, color: Rgb, width: f32) {
        let c = &mut self.page().content;
        c.set_stroke_rgb(color[0], color[1], color[2]);
        c.set_line_width(width);
        c.move_to(x, PAGE_H - y_top);
        c.line_to(x + w, PAGE_H - y_top);
        c.stroke();
    }

    /// Registers an image (flattened onto white) and returns its index.
    pub fn add_image(&mut self, img: &RgbaImage) -> usize {
        let rgb: Vec<u8> = img
            .pixels()
            .flat_map(|p| {
                let a = p.0[3] as u32;
                let f = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
                [f(p.0[0]), f(p.0[1]), f(p.0[2])]
            })
            .collect();
        let idx = self.images.len();
        self.images.push(PendingImage {
            name: format!("Im{}", idx + 1),
            width: img.width(),
            height: img.height(),
            data: compress_to_vec_zlib(&rgb, CompressionLevel::DefaultLevel as u8),
        });
        idx
    }

    /// Draws a registered image in the box (`y_top` from the page top).
    pub fn draw_image(&mut self, idx: usize, x: f32, y_top: f32, w: f32, h: f32) {
        let name = self.images[idx].name.clone();
        let page = self.page();
        if !page.images.contains(&idx) {
            page.images.push(idx);
        }
        page.content.save_state();
        page.content
            .transform([w, 0.0, 0.0, h, x, PAGE_H - y_top - h]);
        page.content.x_object(Name(name.as_bytes()));
        page.content.restore_state();
    }

    // ------------------------------------------------------------ flow

    /// Wrapped text at the left margin (plus `indent`), `width` wide.
    pub fn text_block(
        &mut self,
        s: &str,
        size: f32,
        bold: bool,
        color: Rgb,
        indent: f32,
        width: f32,
    ) {
        let line_h = size * 1.4;
        for line in wrap(s, size, bold, width) {
            self.ensure(line_h);
            let y = self.y;
            self.text_at(MARGIN + indent, y, &line, size, bold, color);
            self.y += line_h;
        }
    }

    pub fn text(&mut self, s: &str, size: f32, bold: bool, color: Rgb, indent: f32) {
        self.text_block(s, size, bold, color, indent, CONTENT_W - indent);
    }

    /// Section title: small accent tag, big title and an optional subtitle.
    pub fn section(&mut self, tag: &str, title: &str, subtitle: &str) {
        self.ensure(90.0);
        if self.y > CONTENT_TOP + 1.0 {
            self.space(14.0);
        }
        let y = self.y;
        self.round_rect(MARGIN, y + 1.0, 4.0, 13.0, 2.0, Some(ACCENT), None);
        self.text_at(
            MARGIN + 10.0,
            y + 2.0,
            &tag.to_uppercase(),
            8.5,
            true,
            ACCENT,
        );
        self.y += 18.0;
        self.text(title, 20.0, true, INK, 0.0);
        if !subtitle.is_empty() {
            self.space(1.0);
            self.text(subtitle, 10.0, false, MUTED, 0.0);
        }
        self.space(10.0);
    }

    /// Smaller heading inside a section.
    pub fn heading(&mut self, s: &str, size: f32) {
        self.ensure(size * 3.5);
        self.space(size * 0.7);
        self.text(s, size, true, INK, 0.0);
        self.space(size * 0.25);
    }

    pub fn paragraph(&mut self, s: &str) {
        self.text(s, 9.5, false, TEXT, 0.0);
        self.space(5.0);
    }

    pub fn bullet(&mut self, s: &str, color: Rgb) {
        let size = 9.5;
        let line_h = size * 1.4;
        for (i, line) in wrap(s, size, false, CONTENT_W - 14.0).iter().enumerate() {
            self.ensure(line_h);
            let y = self.y;
            if i == 0 {
                self.round_rect(MARGIN + 2.0, y + 3.5, 5.0, 5.0, 2.5, Some(color), None);
            }
            self.text_at(MARGIN + 14.0, y, line, size, false, TEXT);
            self.y += line_h;
        }
        self.space(3.0);
    }

    /// Colored box with a left bar, for findings and notes.
    pub fn callout(&mut self, s: &str, color: Rgb) {
        let size = 9.5;
        let line_h = size * 1.4;
        let lines = wrap(s, size, false, CONTENT_W - 30.0);
        let h = lines.len() as f32 * line_h + 14.0;
        self.ensure(h + 6.0);
        let y = self.y;
        let tint = [
            1.0 - (1.0 - color[0]) * 0.1,
            1.0 - (1.0 - color[1]) * 0.1,
            1.0 - (1.0 - color[2]) * 0.1,
        ];
        self.round_rect(MARGIN, y, CONTENT_W, h, 6.0, Some(tint), None);
        self.round_rect(MARGIN, y, 4.0, h, 2.0, Some(color), None);
        for (i, line) in lines.iter().enumerate() {
            self.text_at(
                MARGIN + 16.0,
                y + 7.0 + i as f32 * line_h,
                line,
                size,
                false,
                TEXT,
            );
        }
        self.y += h + 6.0;
    }

    /// Row of KPI tiles.
    pub fn kpis(&mut self, items: &[Kpi]) {
        if items.is_empty() {
            return;
        }
        let gap = 10.0;
        let n = items.len() as f32;
        let w = (CONTENT_W - gap * (n - 1.0)) / n;
        let h = 58.0;
        self.ensure(h + 10.0);
        let y = self.y;
        for (i, k) in items.iter().enumerate() {
            let x = MARGIN + i as f32 * (w + gap);
            self.round_rect(x, y, w, h, 8.0, Some(SURFACE), Some(LINE));
            let size = if k.value.len() > 8 { 15.0 } else { 20.0 };
            self.text_at(x + 12.0, y + 11.0, &k.value, size, true, k.color);
            for (j, line) in wrap(&k.label, 8.5, false, w - 20.0)
                .iter()
                .take(2)
                .enumerate()
            {
                self.text_at(
                    x + 12.0,
                    y + 36.0 + j as f32 * 10.0,
                    line,
                    8.5,
                    false,
                    MUTED,
                );
            }
        }
        self.y += h + 12.0;
    }

    /// Two-column key/value list inside a card.
    pub fn key_values(&mut self, rows: &[(String, String)]) {
        let size = 9.5;
        let line_h = size * 1.45;
        let key_w = rows
            .iter()
            .map(|(k, _)| text_width(k, size, true))
            .fold(0.0, f32::max)
            + 18.0;
        let wrapped: Vec<Vec<String>> = rows
            .iter()
            .map(|(_, v)| wrap(v, size, false, CONTENT_W - key_w - 28.0))
            .collect();
        let h: f32 = wrapped
            .iter()
            .map(|l| l.len() as f32 * line_h + 6.0)
            .sum::<f32>()
            + 14.0;
        self.ensure(h.min(CONTENT_BOTTOM - CONTENT_TOP));
        let top = self.y;
        self.round_rect(MARGIN, top, CONTENT_W, h, 8.0, Some(SURFACE), Some(LINE));
        let mut y = top + 10.0;
        for ((k, _), lines) in rows.iter().zip(&wrapped) {
            self.text_at(MARGIN + 14.0, y, k, size, true, INK);
            for (i, line) in lines.iter().enumerate() {
                self.text_at(
                    MARGIN + 14.0 + key_w,
                    y + i as f32 * line_h,
                    line,
                    size,
                    false,
                    TEXT,
                );
            }
            y += lines.len() as f32 * line_h + 6.0;
        }
        self.y = top + h + 10.0;
    }

    /// Table with an accent header row and zebra rows. Columns get widths
    /// proportional to `weights`. The header repeats after page breaks.
    pub fn table(&mut self, header: &[String], rows: &[Vec<String>], weights: &[f32]) {
        let size = 8.5;
        let line_h = size * 1.35;
        let total: f32 = weights.iter().sum();
        let widths: Vec<f32> = weights.iter().map(|w| CONTENT_W * w / total).collect();
        let measure = |cells: &[String], bold: bool| -> (Vec<Vec<String>>, f32) {
            let wrapped: Vec<Vec<String>> = cells
                .iter()
                .zip(&widths)
                .map(|(c, w)| wrap(c, size, bold, w - 12.0))
                .collect();
            let n = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
            (wrapped, n as f32 * line_h + 10.0)
        };
        let draw = |doc: &mut Doc, wrapped: &[Vec<String>], h: f32, kind: u8| {
            let top = doc.y;
            match kind {
                0 => doc.round_rect(MARGIN, top, CONTENT_W, h, 5.0, Some(INK), None),
                1 => doc.fill_rect(MARGIN, top, CONTENT_W, h, SURFACE),
                _ => {}
            }
            let mut x = MARGIN;
            for (lines, w) in wrapped.iter().zip(&widths) {
                for (i, line) in lines.iter().enumerate() {
                    let (bold, color) = if kind == 0 {
                        (true, WHITE)
                    } else {
                        (false, TEXT)
                    };
                    doc.text_at(
                        x + 6.0,
                        top + 5.0 + i as f32 * line_h,
                        line,
                        size,
                        bold,
                        color,
                    );
                }
                x += w;
            }
            if kind != 0 {
                doc.hline(MARGIN, top + h, CONTENT_W, LINE, 0.5);
            }
            doc.y += h;
        };
        let (head, head_h) = measure(header, true);
        self.ensure(head_h * 2.5);
        draw(self, &head, head_h, 0);
        for (i, r) in rows.iter().enumerate() {
            let (cells, h) = measure(r, false);
            if self.y + h > CONTENT_BOTTOM {
                self.new_page();
                draw(self, &head, head_h, 0);
            }
            draw(self, &cells, h, if i % 2 == 1 { 1 } else { 2 });
        }
        self.space(10.0);
    }

    /// Size of `img` placed within `max_w × max_h`.
    pub fn fit(img: &RgbaImage, max_w: f32, max_h: f32) -> (f32, f32) {
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let k = (max_w / iw).min(max_h / ih);
        (iw * k, ih * k)
    }

    /// A card with a title, a one-line explanation and the image, kept on
    /// one page.
    pub fn figure_card(&mut self, title: &str, caption: &str, img: &RgbaImage, max_h: f32) {
        let pad = 12.0;
        let inner_w = CONTENT_W - 2.0 * pad;
        let caption_lines = if caption.is_empty() {
            Vec::new()
        } else {
            wrap(caption, 8.5, false, inner_w)
        };
        let head = 16.0 + caption_lines.len() as f32 * 11.5 + 6.0;
        let (w, h) = Self::fit(img, inner_w, max_h);
        let total = head + h + 2.0 * pad;
        self.ensure(total + 8.0);
        let top = self.y;
        self.round_rect(MARGIN, top, CONTENT_W, total, 10.0, Some(WHITE), Some(LINE));
        self.text_at(MARGIN + pad, top + pad, title, 11.5, true, INK);
        for (i, line) in caption_lines.iter().enumerate() {
            self.text_at(
                MARGIN + pad,
                top + pad + 16.0 + i as f32 * 11.5,
                line,
                8.5,
                false,
                MUTED,
            );
        }
        let idx = self.add_image(img);
        let x = MARGIN + pad + (inner_w - w) / 2.0;
        self.draw_image(idx, x, top + pad + head, w, h);
        self.y = top + total + 10.0;
    }

    /// Backwards-compatible plain figure (title + image).
    pub fn figure(&mut self, caption: &str, img: &RgbaImage, max_h: f32) {
        self.figure_card(caption, "", img, max_h);
    }

    /// Image scaled to the text width (and `max_h` points).
    pub fn image(&mut self, img: &RgbaImage, max_h: f32) {
        let (w, h) = Self::fit(img, CONTENT_W, max_h);
        self.ensure(h + 4.0);
        let idx = self.add_image(img);
        let y = self.y;
        self.draw_image(idx, MARGIN + (CONTENT_W - w) / 2.0, y, w, h);
        self.y += h + 6.0;
    }

    fn decorate(&mut self) {
        let n = self.pages.len();
        let footer = self.footer.clone();
        let header = self.header.clone();
        let logo = self.logo;
        let pages = std::mem::take(&mut self.pages);
        for (i, page) in pages.into_iter().enumerate() {
            self.pages.push(page);
            // Footer on every page.
            self.hline(MARGIN, PAGE_H - 44.0, CONTENT_W, LINE, 0.6);
            self.text_at(MARGIN, PAGE_H - 36.0, &footer, 7.5, false, MUTED);
            let num = format!("{} / {}", i + 1, n);
            let w = text_width(&num, 7.5, true);
            self.text_at(PAGE_W - MARGIN - w, PAGE_H - 36.0, &num, 7.5, true, INK);
            if i == 0 {
                continue;
            }
            // Header with the logo, except on the cover.
            if let Some(idx) = logo {
                self.draw_image(idx, MARGIN, 30.0, 18.0, 18.0);
            }
            self.text_at(MARGIN + 24.0, 33.0, "Ondalux Heatmap", 9.5, true, INK);
            let w = text_width(&header, 8.5, false);
            self.text_at(PAGE_W - MARGIN - w, 34.0, &header, 8.5, false, MUTED);
            self.hline(MARGIN, 56.0, CONTENT_W, LINE, 0.6);
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.decorate();
        let n_pages = self.pages.len();
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
    fn produces_a_multi_page_pdf_with_header_logo() {
        let logo = RgbaImage::from_pixel(8, 8, image::Rgba([30, 90, 220, 255]));
        let mut d = Doc::new("Test", "Proyecto", "footer", Some(&logo));
        d.section("Sección", "Título", "Subtítulo");
        d.kpis(&[
            Kpi {
                value: "98 %".into(),
                label: "cobertura".into(),
                color: OK,
            },
            Kpi {
                value: "4".into(),
                label: "APs".into(),
                color: INK,
            },
        ]);
        for i in 0..120 {
            d.paragraph(&format!("Párrafo número {i} con texto de relleno."));
        }
        d.callout("Una recomendación importante.", WARN);
        d.table(
            &["A".into(), "B".into()],
            &(0..80)
                .map(|i| vec![i.to_string(), "x".into()])
                .collect::<Vec<_>>(),
            &[1.0, 2.0],
        );
        d.figure_card(
            "Mapa",
            "Explicación",
            &RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 255])),
            100.0,
        );
        let bytes = d.finish();
        assert!(bytes.starts_with(b"%PDF-"));
        let pdf = hayro::hayro_syntax::Pdf::new(bytes).expect("valid PDF");
        assert!(pdf.pages().len() >= 4);
    }
}
