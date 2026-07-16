//! PDF-generointi. Rustissa ei ole reportlabin `Table`-vastinetta, joten
//! taulukko piirretään käsin: ruudukko viivoina ja solutekstit
//! Helvetica-leveystaulukon avulla keskitettynä.

use crate::dates::next_1st_or_15th_day_fi;
use polars::prelude::*;
use printpdf::{
    BuiltinFont, Color, IndirectFontRef, Line, Mm, PdfDocument, PdfLayerReference, Point, Rgb,
};
use std::fs;
use std::io::BufWriter;
use std::path::Path;

// --- mitat -----------------------------------------------------------------

const PT_PER_MM: f32 = 72.0 / 25.4;
const A4_SHORT_MM: f32 = 210.0;
const A4_LONG_MM: f32 = 297.0;
const LEFT_MARGIN_MM: f32 = 5.0;
const TOP_MARGIN_MM: f32 = 20.0;
/// reportlabin `Frame`-oletuspehmuste (leftPadding/topPadding).
const FRAME_PADDING_PT: f32 = 6.0;
/// reportlabin `Table`-oletuspehmuste (LEFTPADDING).
const CELL_PADDING_PT: f32 = 6.0;

fn pt_to_mm(pt: f32) -> f32 {
    pt / PT_PER_MM
}

// --- Helvetican merkkileveydet (AFM, 1/1000 em) ----------------------------
// Tarvitaan tekstin keskitykseen: printpdf ei tarjoa metriikkaa
// sisäänrakennetuille fonteille. Indeksi 0 = merkki 32 (välilyönti).

#[rustfmt::skip]
const HELVETICA_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556,
    278, 278, 584, 584, 584, 556, 1015,
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667,
    778, 722, 667, 611, 722, 667, 944, 667, 667, 611,
    278, 278, 278, 469, 556, 333,
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556,
    556, 333, 500, 278, 556, 500, 722, 500, 500, 500,
    334, 260, 334, 584,
];

#[rustfmt::skip]
const HELVETICA_BOLD_WIDTHS: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556,
    333, 333, 584, 584, 584, 611, 975,
    722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667,
    778, 722, 667, 611, 722, 667, 944, 667, 667, 611,
    333, 278, 333, 584, 556, 333,
    556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611,
    611, 389, 556, 333, 611, 556, 778, 556, 556, 500,
    389, 280, 389, 584,
];

fn char_width_em(c: char, bold: bool) -> f32 {
    let table = if bold { &HELVETICA_BOLD_WIDTHS } else { &HELVETICA_WIDTHS };
    // Helveticassa aksentilliset merkit ovat yhtä leveitä kuin peruskirjaimet.
    let base = match c {
        'Ä' | 'Å' => 'A',
        'ä' | 'å' => 'a',
        'Ö' => 'O',
        'ö' => 'o',
        'Ü' => 'U',
        'ü' => 'u',
        'É' => 'E',
        'é' => 'e',
        other => other,
    };
    let code = base as u32;
    if (32..=126).contains(&code) {
        f32::from(table[(code - 32) as usize]) / 1000.0
    } else {
        0.556
    }
}

fn text_width_pt(text: &str, font_size: f32, bold: bool) -> f32 {
    text.chars().map(|c| char_width_em(c, bold)).sum::<f32>() * font_size
}

// --- DataFrame -> merkkijonot ----------------------------------------------

/// Python `str(float)` tuottaa kokonaisluvuille ".0"-päätteen, Rustin
/// `Display` ei. Muuten molemmat käyttävät lyhintä edestakaisin
/// palautuvaa esitystä.
fn python_float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf".to_string() } else { "-inf".to_string() };
    }
    let formatted = format!("{value}");
    if formatted.contains('.') || formatted.contains('e') {
        formatted
    } else {
        format!("{formatted}.0")
    }
}

fn any_value_to_string(value: AnyValue) -> String {
    match value {
        // pandasissa puuttuva arvo on float NaN, ja str(nan) == "nan".
        AnyValue::Null => "nan".to_string(),
        AnyValue::String(v) => v.to_string(),
        AnyValue::StringOwned(v) => v.to_string(),
        AnyValue::Boolean(v) => if v { "True" } else { "False" }.to_string(),
        AnyValue::Float32(v) => python_float_repr(f64::from(v)),
        AnyValue::Float64(v) => python_float_repr(v),
        other => other.to_string(),
    }
}

fn dataframe_to_strings(df: &DataFrame) -> PolarsResult<(Vec<String>, Vec<Vec<String>>)> {
    let headers: Vec<String> = df.get_columns().iter().map(|c| c.name().to_string()).collect();
    let mut rows = Vec::with_capacity(df.height());
    for row_idx in 0..df.height() {
        let mut row = Vec::with_capacity(df.width());
        for column in df.get_columns() {
            row.push(any_value_to_string(column.as_materialized_series().get(row_idx)?));
        }
        rows.push(row);
    }
    Ok((headers, rows))
}

/// Python: `custom_widths.get(col, max(pisin_arvo, len(col)) * 2 * mm)`.
fn compute_column_widths(
    names: &[String],
    rows: &[Vec<String>],
    custom: &[(&str, f32)],
) -> Vec<f32> {
    names
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            if let Some((_, width)) = custom.iter().find(|(key, _)| key == name) {
                return *width;
            }
            let longest = rows
                .iter()
                .filter_map(|row| row.get(idx))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0);
            longest.max(name.chars().count()) as f32 * 2.0
        })
        .collect()
}

fn index_of(names: &[String], target: &str) -> Option<usize> {
    names.iter().position(|n| n == target)
}

// --- taulukon renderöinti ---------------------------------------------------

struct TableReport {
    landscape: bool,
    page_info: String,
    /// Sivuotsikon y-koordinaatti pisteinä. Python-versiossa tämä on laskettu
    /// eri tavalla joka raportissa, joten arvo annetaan erikseen.
    info_y_pt: f32,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    col_widths_mm: Vec<f32>,
    header_height_mm: f32,
    body_height_mm: f32,
    header_font_size: f32,
    body_font_size: f32,
    max_rows_per_page: usize,
    spacer_mm: f32,
    left_align_column: Option<usize>,
}

impl TableReport {
    fn render(&self, export_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        if Path::new(export_path).exists() {
            fs::remove_file(export_path)?;
        }

        let (page_w, page_h) = if self.landscape {
            (A4_LONG_MM, A4_SHORT_MM)
        } else {
            (A4_SHORT_MM, A4_LONG_MM)
        };

        // Python: data = [header] + rivit, paloitellaan, ja otsikko lisätään
        // erikseen jokaisen sivun eteen paitsi ensimmäisen. Tämän vuoksi
        // ensimmäisellä sivulla on yksi datarivi vähemmän kuin muilla.
        let mut data: Vec<&Vec<String>> = Vec::with_capacity(self.rows.len() + 1);
        data.push(&self.headers);
        data.extend(self.rows.iter());

        let pages: Vec<Vec<&Vec<String>>> = data
            .chunks(self.max_rows_per_page)
            .enumerate()
            .map(|(idx, chunk)| {
                let mut page = Vec::with_capacity(chunk.len() + 1);
                if idx != 0 {
                    page.push(&self.headers);
                }
                page.extend(chunk.iter().copied());
                page
            })
            .collect();

        let (doc, first_page, first_layer) =
            PdfDocument::new(self.page_info.clone(), Mm(page_w), Mm(page_h), "Layer 1");
        let regular = doc.add_builtin_font(BuiltinFont::Helvetica)?;
        let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;

        for (page_idx, page_rows) in pages.iter().enumerate() {
            let layer = if page_idx == 0 {
                doc.get_page(first_page).get_layer(first_layer)
            } else {
                let (page, layer) = doc.add_page(Mm(page_w), Mm(page_h), "Layer 1");
                doc.get_page(page).get_layer(layer)
            };
            self.draw_page(&layer, page_rows, page_h, &regular, &bold);
        }

        doc.save(&mut BufWriter::new(fs::File::create(export_path)?))?;
        println!("PDF successfully created and saved at {export_path}");
        Ok(())
    }

    fn draw_page(
        &self,
        layer: &PdfLayerReference,
        rows: &[&Vec<String>],
        page_h: f32,
        regular: &IndirectFontRef,
        bold: &IndirectFontRef,
    ) {
        // Sivuotsikko, vastaa reportlabin canvas.drawString-kutsua.
        layer.use_text(
            self.page_info.clone(),
            8.0,
            Mm(LEFT_MARGIN_MM),
            Mm(pt_to_mm(self.info_y_pt)),
            regular,
        );

        let left = LEFT_MARGIN_MM + pt_to_mm(FRAME_PADDING_PT);
        let table_top = page_h - TOP_MARGIN_MM - pt_to_mm(FRAME_PADDING_PT) - self.spacer_mm;

        let row_heights: Vec<f32> = (0..rows.len())
            .map(|idx| if idx == 0 { self.header_height_mm } else { self.body_height_mm })
            .collect();
        let total_width: f32 = self.col_widths_mm.iter().sum();
        let total_height: f32 = row_heights.iter().sum();

        layer.set_outline_thickness(0.5);
        layer.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));

        let mut y = table_top;
        draw_line(layer, left, y, left + total_width, y);
        for height in &row_heights {
            y -= height;
            draw_line(layer, left, y, left + total_width, y);
        }

        let mut x = left;
        draw_line(layer, x, table_top, x, table_top - total_height);
        for width in &self.col_widths_mm {
            x += width;
            draw_line(layer, x, table_top, x, table_top - total_height);
        }

        let mut row_top = table_top;
        for (row_idx, row) in rows.iter().enumerate() {
            let height = row_heights[row_idx];
            let is_header = row_idx == 0;
            let font_size = if is_header { self.header_font_size } else { self.body_font_size };
            let font = if is_header { bold } else { regular };

            let mut cell_x = left;
            for (col_idx, cell) in row.iter().enumerate() {
                let width = self.col_widths_mm.get(col_idx).copied().unwrap_or(0.0);
                self.draw_cell(
                    layer,
                    cell,
                    cell_x,
                    row_top,
                    width,
                    height,
                    font_size,
                    font,
                    is_header,
                    self.left_align_column == Some(col_idx),
                );
                cell_x += width;
            }
            row_top -= height;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_cell(
        &self,
        layer: &PdfLayerReference,
        text: &str,
        cell_x: f32,
        cell_top: f32,
        cell_width: f32,
        cell_height: f32,
        font_size: f32,
        font: &IndirectFontRef,
        bold: bool,
        left_align: bool,
    ) {
        // reportlab jakaa taulukkosolun tekstin rivinvaihdoissa.
        let lines: Vec<&str> = text.split('\n').collect();
        let leading = font_size * 1.2; // reportlabin oletus
        let block_height_mm = pt_to_mm(lines.len() as f32 * leading);
        let block_top = (cell_top - cell_height / 2.0) + block_height_mm / 2.0;

        for (line_idx, line) in lines.iter().enumerate() {
            // VALIGN=MIDDLE: rivit keskitetään soluun, perusviiva rivin alalaidan yläpuolelle.
            let baseline = block_top - pt_to_mm(leading * (line_idx as f32 + 1.0 - 0.25));
            let x = if left_align {
                cell_x + pt_to_mm(CELL_PADDING_PT)
            } else {
                cell_x + (cell_width - pt_to_mm(text_width_pt(line, font_size, bold))) / 2.0
            };
            layer.use_text(line.to_string(), font_size, Mm(x), Mm(baseline), font);
        }
    }
}

fn draw_line(layer: &PdfLayerReference, x1: f32, y1: f32, x2: f32, y2: f32) {
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(x1), Mm(y1)), false),
            (Point::new(Mm(x2), Mm(y2)), false),
        ],
        is_closed: false,
    });
}

// --- raporttikohtaiset kääröt ----------------------------------------------

const HINNASTO_WIDTHS_COMMON: &[(&str, f32)] = &[
    ("VNR", 8.0),
    ("THsALV", 10.0),
    ("VHsALV", 10.0),
    ("VHcALV", 10.0),
    ("Pakkauskoko", 20.0),
    ("Reseptivalmiste", 18.0),
    ("Tukkuhinta erotus (%)", 25.0),
    ("Tukkuhinta erotus (eur)", 25.0),
    ("THsALV (ed.taksa)", 25.0),
    ("Apteekkiveroperuste", 25.0),
];

fn hinnasto_widths(pitka_tuotenimi_mm: f32) -> Vec<(&'static str, f32)> {
    let mut widths = HINNASTO_WIDTHS_COMMON.to_vec();
    widths.push(("Pitkä tuotenimi", pitka_tuotenimi_mm));
    widths
}

pub fn create_reseptilaakkeiden_hinnasto_portrait_pdf(
    df: &DataFrame,
    export_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (headers, rows) = dataframe_to_strings(df)?;
    let widths = compute_column_widths(&headers, &rows, &hinnasto_widths(110.0));
    TableReport {
        landscape: false,
        page_info: format!("Reseptilääkkeiden hinnasto {}", next_1st_or_15th_day_fi()),
        // Python: page_size[1] - 20, missä page_size[1] = A4[1] - 10mm
        info_y_pt: 841.8898 - 10.0 * PT_PER_MM - 20.0,
        left_align_column: index_of(&headers, "Pitkä tuotenimi"),
        headers,
        rows,
        col_widths_mm: widths,
        header_height_mm: 5.0,
        body_height_mm: 5.0,
        header_font_size: 6.0,
        body_font_size: 5.0,
        max_rows_per_page: 50,
        spacer_mm: 3.0,
    }
    .render(export_path)
}

pub fn create_itsehoitolaakkeiden_hinnasto_portrait_pdf(
    df: &DataFrame,
    export_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (headers, rows) = dataframe_to_strings(df)?;
    let widths = compute_column_widths(&headers, &rows, &hinnasto_widths(115.0));
    TableReport {
        landscape: false,
        page_info: format!("Itsehoitolääkkeiden hinnasto {}", next_1st_or_15th_day_fi()),
        info_y_pt: 841.8898 - 10.0 * PT_PER_MM - 20.0,
        left_align_column: index_of(&headers, "Pitkä tuotenimi"),
        headers,
        rows,
        col_widths_mm: widths,
        header_height_mm: 5.0,
        body_height_mm: 5.0,
        header_font_size: 6.0,
        body_font_size: 5.0,
        max_rows_per_page: 50,
        spacer_mm: 3.0,
    }
    .render(export_path)
}

fn transform_header(name: &str) -> String {
    match name {
        "THsALV (ed.taksa)" => "THsALV\n(ed.taksa)",
        "Hintamuutos" => "Hinta-\nmuutos",
        "Muutostieto" => "Muutos-\ntieto",
        "Kelakorvattava" => "Kela-\nkorvattava",
        "Reseptivalmiste" => "Resepti-\nvalmiste",
        "Tukkuhinta erotus (%)" => "Tukkuhinta\nerotus (%)",
        "Tukkuhinta erotus (eur)" => "Tukkuhinta\nerotus (eur)",
        other => other,
    }
    .to_string()
}

pub fn create_hintamuutoslista_laakevalmisteet_horizontal_pdf(
    df: &DataFrame,
    export_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (original_headers, rows) = dataframe_to_strings(df)?;
    // Python laskee leveydet alkuperäisillä nimillä mutta piirtää muunnetut otsikot.
    let widths = compute_column_widths(
        &original_headers,
        &rows,
        &[
            ("VNR", 8.0),
            ("THsALV", 10.0),
            ("VHsALV", 10.0),
            ("VHcALV", 10.0),
            ("Pakkauskoko", 16.0),
            ("Kelakorvattava", 13.0),
            ("Hintamuutos", 9.0),
            ("Reseptivalmiste", 10.0),
            ("Myyntiluvan haltija", 45.0),
            ("Tukku", 14.0),
            ("THsALV (ed.taksa)", 13.0),
            ("Tukkuhinta erotus (%)", 13.0),
            ("Tukkuhinta erotus (eur)", 13.0),
            ("Pitkä tuotenimi", 105.0),
        ],
    );
    TableReport {
        landscape: true,
        page_info: format!("Hintamuutoslista lääkevalmisteet {}", next_1st_or_15th_day_fi()),
        // Python: A4[0] - 20*mm
        info_y_pt: 595.2756 - 20.0 * PT_PER_MM,
        left_align_column: index_of(&original_headers, "Pitkä tuotenimi"),
        headers: original_headers.iter().map(|h| transform_header(h)).collect(),
        rows,
        col_widths_mm: widths,
        header_height_mm: 9.0,
        body_height_mm: 4.5,
        header_font_size: 6.0,
        body_font_size: 6.0,
        max_rows_per_page: 35,
        spacer_mm: 3.0,
    }
    .render(export_path)
}

pub fn create_muutoslista_laakevalmisteet_horizontal_pdf(
    df: &DataFrame,
    export_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (original_headers, rows) = dataframe_to_strings(df)?;
    // Python nimeää sarakkeet uudelleen ENNEN leveyslaskentaa, joten
    // hakuavaimet ovat tässä muunnettuja nimiä.
    let headers: Vec<String> = original_headers.iter().map(|h| transform_header(h)).collect();
    let widths = compute_column_widths(
        &headers,
        &rows,
        &[
            ("VNR", 8.0),
            ("Pitkä tuotenimi", 100.0),
            ("Pakkauskoko", 20.0),
            ("THsALV", 13.0),
            ("VHsALV", 13.0),
            ("VHcALV", 13.0),
            ("Kela-\nkorvattava", 15.0),
            ("Muutos-\ntieto", 13.0),
            ("Hinta-\nmuutos", 13.0),
            ("Resepti-\nvalmiste", 15.0),
            ("Myyntiluvan haltija", 50.0),
            ("Tukku", 16.0),
        ],
    );
    TableReport {
        landscape: true,
        page_info: format!("Muutoslista lääkevalmisteet {}", next_1st_or_15th_day_fi()),
        // Python: landscape(A4)[1] - 10*mm
        info_y_pt: 595.2756 - 10.0 * PT_PER_MM,
        left_align_column: index_of(&headers, "Pitkä tuotenimi"),
        headers,
        rows,
        col_widths_mm: widths,
        header_height_mm: 9.0,
        body_height_mm: 4.5,
        header_font_size: 8.0,
        body_font_size: 6.0,
        max_rows_per_page: 35,
        // Tämä on ainoa raportti, jossa Python EI lisää Spaceria.
        spacer_mm: 0.0,
    }
    .render(export_path)
}

pub fn create_poistolista_laakevalmisteet_horizontal_pdf(
    df: &DataFrame,
    export_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (headers, rows) = dataframe_to_strings(df)?;
    let widths = compute_column_widths(
        &headers,
        &rows,
        &[
            ("VNR", 10.0),
            ("THsALV", 10.0),
            ("VHsALV", 10.0),
            ("VHcALV", 10.0),
            ("Pakkauskoko", 25.0),
            ("Kelakorvattava", 16.0),
            ("Hintamuutos", 15.0),
            ("Muutostieto", 20.0),
            ("Reseptivalmiste", 23.0),
            ("Myyntiluvan haltija", 50.0),
            ("Tukku", 16.0),
            ("THsALV (ed.taksa)", 23.0),
            ("Tukkuhinta erotus (%)", 23.0),
            ("Tukkuhinta erotus (eur)", 23.0),
            ("Pitkä tuotenimi", 120.0),
        ],
    );
    TableReport {
        landscape: true,
        page_info: format!(
            "Poistolista lääkevalmisteet {}  Huom! Muutostieto-sarake poistettu, koska kaikilla riveillä se on PP.",
            next_1st_or_15th_day_fi()
        ),
        // Python: page_size[1] - 20, missä page_size[1] = A4[0] - 10mm
        info_y_pt: 595.2756 - 10.0 * PT_PER_MM - 20.0,
        left_align_column: index_of(&headers, "Pitkä tuotenimi"),
        headers,
        rows,
        col_widths_mm: widths,
        header_height_mm: 5.0,
        body_height_mm: 5.0,
        header_font_size: 8.0,
        body_font_size: 6.0,
        max_rows_per_page: 35,
        spacer_mm: 3.0,
    }
    .render(export_path)
}
