//! Akeneo taksaraportti -ohjelma.
//! Rust-käännös alkuperäisestä Python-versiosta.

mod dates;
mod excel;
mod pdf;
mod transform;

use polars::prelude::*;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

const HOME_FOLDER: &str = r"C:\Akeneo_Taksaraportti_ohjelma";

/// Sarakkeet, jotka import-CSV:ssä on vähintään oltava. Ylimääräiset
/// sarakkeet ovat sallittuja.
const REQUIRED_COLUMNS: &[&str] = &[
    "yksilointitunnus",
    "kauppanimi",
    "vahvuus",
    "pakkauskoko_teksti",
    "laite",
    "laakemuoto_fi_FI",
    "tukkuhinta",
    "tukkuhinta_edellinen_taksa",
    "verotonhinta",
    "verollinenmyyntihinta",
    "versionumero",
    "muutostieto_ltk",
    "hintamuutos_ltk",
    "korvausluokka",
    "reseptistatus",
    "myyntiluvan_haltija",
    "toimittajat",
    "modified_external_kela_integration",
    "laakemuoto_koodi_ja_selite",
    "apteekkiveroperuste",
    "markkinoija_ltk",
    "poistumassa_ltk",
    "hintaputki_ylin_hinta",
    "viitehinta",
    "tuotteen_tila",
];

fn print_separator() {
    println!("####################################");
}

fn create_folder_if_missing(folder: &Path) {
    if fs::create_dir(folder).is_ok() {
        // Python `print("Luodaan kansio ", folder)` tuottaa kaksi välilyöntiä.
        println!("Luodaan kansio  {}", folder.display());
        print_separator();
    }
}

fn clear_folder(folder: &Path) {
    if !folder.exists() {
        println!("The folder path does not exist.");
        return;
    }
    match fs::read_dir(folder) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                let result = if path.is_dir() && !path.is_symlink() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                if let Err(e) = result {
                    println!("Failed to delete {}. Reason: {e}", path.display());
                }
            }
        }
        Err(e) => println!("Failed to read {}. Reason: {e}", folder.display()),
    }
    println!("{} Folder cleared successfully.", folder.display());
}

/// Tarkistaa, että import-CSV sisältää kaikki pakolliset sarakkeet.
/// Otsikkorivi luetaan omana kevyenä lukunaan (ei rivien jäsennystä), koska
/// varsinainen luku kaatuisi polarsin omaan virheeseen jo `schema_overwrite`
/// -vaiheessa, jos `yksilointitunnus` puuttuu — silloin käyttäjä ei näkisi
/// tätä viestiä.
fn validate_csv_columns(file_path: &Path) -> PolarsResult<()> {
    let header = CsvReadOptions::default()
        .with_has_header(true)
        .with_infer_schema_length(Some(0))
        .with_n_rows(Some(0))
        .with_parse_options(
            CsvParseOptions::default()
                .with_separator(b'|')
                .with_encoding(CsvEncoding::Utf8),
        )
        .try_into_reader_with_file_path(Some(file_path.to_path_buf()))?
        .finish()?;

    let missing: Vec<&str> = REQUIRED_COLUMNS
        .iter()
        .copied()
        .filter(|required| {
            !header
                .get_columns()
                .iter()
                .any(|column| column.name().as_str() == *required)
        })
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    println!("Input file schema is invalid");
    println!("Tiedostosta puuttuu {} pakollista saraketta:", missing.len());
    for name in &missing {
        println!("  - {name}");
    }
    println!("Korjaa CSV-tiedosto ja käynnistä ohjelma uudelleen.");
    sleep(Duration::from_secs(3));
    exit(0);
}

fn read_csv_file(import_folder: &Path) -> Result<DataFrame, Box<dyn Error>> {
    let mut csv_files: Vec<PathBuf> = fs::read_dir(import_folder)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.to_string_lossy().to_lowercase() == "csv")
        })
        .collect();
    csv_files.sort();

    match csv_files.len() {
        1 => {}
        0 => {
            println!("Import-kansiossa ei ole yhtään CSV-tiedostoa. Lisää CSV ja käynnistä ohjelma uudelleen.");
            sleep(Duration::from_secs(3));
            exit(0);
        }
        _ => {
            println!("Import-kansiossa on yli yksi CSV-tiedosto. Poista ylimääräiset ja käynnistä ohjelma uudelleen.");
            sleep(Duration::from_secs(3));
            exit(0);
        }
    }

    let file_path = csv_files.remove(0);

    validate_csv_columns(&file_path)?;

    // `yksilointitunnus` luetaan merkkijonona, kuten pandasin dtype-parametrissa.
    let schema_overwrite = Schema::from_iter([Field::new(
        "yksilointitunnus".into(),
        DataType::String,
    )]);
    let null_values = NullValues::AllColumns(
        transform::PANDAS_NA_VALUES
            .iter()
            .map(|value| PlSmallStr::from_str(value))
            .collect(),
    );

    let df = CsvReadOptions::default()
        .with_has_header(true)
        .with_infer_schema_length(None)
        .with_schema_overwrite(Some(Arc::new(schema_overwrite)))
        .with_parse_options(
            CsvParseOptions::default()
                .with_separator(b'|')
                .with_encoding(CsvEncoding::Utf8)
                .with_null_values(Some(null_values)),
        )
        .try_into_reader_with_file_path(Some(file_path.clone()))?
        .finish()?;

    println!(
        "Reading file: {}",
        file_path.file_name().unwrap_or_default().to_string_lossy()
    );
    Ok(df)
}

/// pandas: `df[df[column] != value]` säilyttää NaN-rivit (NaN != x on True).
fn filter_not_equal(df: DataFrame, column: &str, value: &str) -> PolarsResult<DataFrame> {
    df.lazy()
        .filter(col(column).neq(lit(value)).fill_null(true))
        .collect()
}

/// pandas: `df[df[column].isin(values)]` pudottaa NaN-rivit.
fn filter_is_in(df: DataFrame, column: &str, values: &[&str]) -> PolarsResult<DataFrame> {
    let allowed = Series::new("allowed".into(), values);
    df.lazy()
        .filter(col(column).is_in(lit(allowed)).fill_null(false))
        .collect()
}

fn export_path(export_folder: &Path, prefix: &str, extension: &str) -> PathBuf {
    export_folder.join(format!(
        "{prefix}_{}.{extension}",
        dates::next_1st_or_15th_day_compact()
    ))
}

fn main() -> Result<(), Box<dyn Error>> {
    let home_folder = PathBuf::from(HOME_FOLDER);
    let import_folder = home_folder.join("IMPORT_kansio");
    let export_folder = home_folder.join("EXPORT_kansio");

    create_folder_if_missing(&home_folder);
    create_folder_if_missing(&import_folder);
    create_folder_if_missing(&export_folder);

    println!("aloitetaan");
    println!("putsataan export-kansio");
    clear_folder(&export_folder);
    print_separator();

    println!("luetaan raakadata");
    let mut df = read_csv_file(&import_folder)?;
    print_separator();

    // --- datan jalostus ----------------------------------------------------

    // poistetaan ei-uusimmat versionumerot
    df = transform::remove_items_which_do_not_have_the_newest_versionumero(df)?;
    // poistetaan lääkkeelliset kaasut
    df = transform::remove_rows_which_have_non_allowed_laakemuoto(df)?;
    // poistetaan rivit, joissa yksilointitunnus on tyhjä/null
    df = transform::remove_rows_where_yksilointitunnus_is_empty(df)?;
    // asetetaan PP-rivien hintatiedot nollaksi ja korvausluokka tyhjäksi
    df = transform::pp_row_refining(df)?;
    // poistetaan null-string-arvot
    df = transform::remove_null_strings_from_columns(df)?;
    // asetetaan reseptistatus Kyllä/Ei -arvoiksi
    df = transform::refine_reseptistatus(df)?;
    // pakotetaan kauppanimi-sarake uppercaseksi
    df = transform::force_string_columns_to_uppercase(df)?;
    // luodaan pitka_tuotenimi-sarake ja järjestetään data sen perusteella
    df = transform::create_pitka_tuotenimi_column(df)?;
    df = transform::sort_dataframe_based_on_columns(df)?;
    df = transform::cut_column_string_length(df, "pitka_tuotenimi", 100)?;
    df = transform::cut_column_string_length(df, "pakkauskoko_teksti", 15)?;
    // tukkuhinnan erotus euroina ja prosentteina
    df = transform::add_tukkuhinta_erotus_columns(df)?;
    df = transform::round_float_column(df, "tukkuhinta_erotus_prosentti", 2)?;
    df = transform::round_float_column(df, "tukkuhinta_erotus_eur", 2)?;
    // tukku-sarake toimittajat-rakenteen perusteella
    df = transform::assign_tukku(df)?;
    // Kelakorvattava-sarake korvausluokka-rakenteen perusteella
    df = transform::add_kelakorvattava_column(df)?;
    // poistetaan sarakkeet, joita ei tarvita finaalirapsaan
    df = transform::drop_unused_columns(df);
    df = transform::rename_columns_for_final_report(df)?;
    df = transform::refine_apteekkiveroperuste_column(df)?;

    // --- reseptilääkkeiden hinnasto ----------------------------------------

    print_separator();
    println!("luodaan reseptilaakkeiden hinnasto");
    let mut report = df.select([
        "VNR",
        "Pitkä tuotenimi",
        "Pakkauskoko",
        "THsALV",
        "VHsALV",
        "VHcALV",
        "Reseptivalmiste",
        "Muutostieto",
        "Apteekkiveroperuste",
    ])?;
    report = filter_not_equal(report, "Reseptivalmiste", "Ei")?;
    report = filter_not_equal(report, "Muutostieto", "PP")?;
    report = report.select([
        "VNR",
        "Pitkä tuotenimi",
        "Pakkauskoko",
        "THsALV",
        "VHsALV",
        "VHcALV",
        "Apteekkiveroperuste",
    ])?;
    excel::write_datapandas_to_file(
        &report,
        &export_path(&export_folder, "Reseptilaakkeiden_hinnasto", "xlsx").to_string_lossy(),
    );
    pdf::create_reseptilaakkeiden_hinnasto_portrait_pdf(
        &report,
        &export_path(&export_folder, "Reseptilaakkeiden_hinnasto", "pdf").to_string_lossy(),
    )?;

    print_separator();
    sleep(Duration::from_secs(1));

    // --- itsehoitolääkkeiden hinnasto --------------------------------------

    print_separator();
    println!("luodaan itsehoitolääkkeiden hinnasto");
    let mut report = df.select([
        "VNR",
        "Pitkä tuotenimi",
        "Pakkauskoko",
        "THsALV",
        "VHsALV",
        "VHcALV",
        "Reseptivalmiste",
        "Muutostieto",
        "Apteekkiveroperuste",
    ])?;
    report = filter_not_equal(report, "Reseptivalmiste", "Kyllä")?;
    report = filter_not_equal(report, "Muutostieto", "PP")?;
    report = report.select([
        "VNR",
        "Pitkä tuotenimi",
        "Pakkauskoko",
        "THsALV",
        "VHsALV",
        "VHcALV",
        "Apteekkiveroperuste",
    ])?;
    excel::write_datapandas_to_file(
        &report,
        &export_path(&export_folder, "Itsehoitolaakkeiden_hinnasto", "xlsx").to_string_lossy(),
    );
    pdf::create_itsehoitolaakkeiden_hinnasto_portrait_pdf(
        &report,
        &export_path(&export_folder, "Itsehoitolaakkeiden_hinnasto", "pdf").to_string_lossy(),
    )?;

    print_separator();
    sleep(Duration::from_secs(1));

    // --- hintamuutoslista lääkevalmisteet ----------------------------------

    print_separator();
    println!("luodaan hintamuutoslista lääkevalmisteet");
    let mut report = df.select([
        "VNR",
        "Pitkä tuotenimi",
        "Pakkauskoko",
        "THsALV (ed.taksa)",
        "Tukkuhinta erotus (eur)",
        "Tukkuhinta erotus (%)",
        "THsALV",
        "VHsALV",
        "VHcALV",
        "Kelakorvattava",
        "Hintamuutos",
        "Reseptivalmiste",
        "Myyntiluvan haltija",
        "Tukku",
        "Muutostieto",
    ])?;
    report = filter_is_in(report, "Hintamuutos", &["HN", "HL"])?;
    report = filter_is_in(report, "Muutostieto", &["MM", "null", ""])?;
    report = report.select([
        "VNR",
        "Pitkä tuotenimi",
        "Pakkauskoko",
        "THsALV (ed.taksa)",
        "Tukkuhinta erotus (eur)",
        "Tukkuhinta erotus (%)",
        "THsALV",
        "VHsALV",
        "VHcALV",
        "Kelakorvattava",
        "Hintamuutos",
        "Reseptivalmiste",
        "Myyntiluvan haltija",
        "Tukku",
    ])?;
    excel::write_datapandas_to_file(
        &report,
        &export_path(&export_folder, "Hintamuutoslista_laakevalmisteet", "xlsx").to_string_lossy(),
    );
    pdf::create_hintamuutoslista_laakevalmisteet_horizontal_pdf(
        &report,
        &export_path(&export_folder, "Hintamuutoslista_laakevalmisteet", "pdf").to_string_lossy(),
    )?;

    print_separator();
    sleep(Duration::from_secs(1));

    // --- muutoslista lääkevalmisteet ---------------------------------------

    print_separator();
    println!("luodaan muutoslista lääkevalmisteet");
    let mut report = df.select([
        "VNR",
        "Pitkä tuotenimi",
        "Pakkauskoko",
        "THsALV",
        "VHsALV",
        "VHcALV",
        "Kelakorvattava",
        "Muutostieto",
        "Hintamuutos",
        "Reseptivalmiste",
        "Myyntiluvan haltija",
        "Tukku",
    ])?;
    report = filter_is_in(report, "Muutostieto", &["MM", "UU"])?;
    excel::write_datapandas_to_file(
        &report,
        &export_path(&export_folder, "Muutoslista_laakevalmisteet", "xlsx").to_string_lossy(),
    );
    pdf::create_muutoslista_laakevalmisteet_horizontal_pdf(
        &report,
        &export_path(&export_folder, "Muutoslista_laakevalmisteet", "pdf").to_string_lossy(),
    )?;

    print_separator();
    sleep(Duration::from_secs(1));

    // --- poistolista lääkevalmisteet ---------------------------------------

    print_separator();
    println!("luodaan poistolista lääkevalmisteet");
    let mut report = df.select([
        "VNR",
        "Pitkä tuotenimi",
        "Pakkauskoko",
        "Muutostieto",
        "Reseptivalmiste",
        "Myyntiluvan haltija",
        "Tukku",
    ])?;
    report = filter_is_in(report, "Muutostieto", &["PP"])?;
    report = report.select([
        "VNR",
        "Pitkä tuotenimi",
        "Pakkauskoko",
        "Reseptivalmiste",
        "Myyntiluvan haltija",
        "Tukku",
    ])?;
    excel::write_datapandas_to_file(
        &report,
        &export_path(&export_folder, "Poistolista_laakevalmisteet", "xlsx").to_string_lossy(),
    );
    pdf::create_poistolista_laakevalmisteet_horizontal_pdf(
        &report,
        &export_path(&export_folder, "Poistolista_laakevalmisteet", "pdf").to_string_lossy(),
    )?;

    print_separator();
    sleep(Duration::from_secs(1));

    // --- lopetus -----------------------------------------------------------

    print_separator();
    println!("Ohjelma suoritettu onnistuneesti");
    print_separator();
    for step in (1..=5).rev() {
        println!("suljetaan ohjelma {step} sekunnin sisällä");
        sleep(Duration::from_secs(1));
    }
    Ok(())
}
