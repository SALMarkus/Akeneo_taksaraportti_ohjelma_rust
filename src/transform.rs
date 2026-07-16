//! Datamuunnokset. Vastaa Python-version funktioita rivi riviltä.

use polars::prelude::*;
use std::collections::HashMap;

/// pandas `read_csv` -oletusarvot, jotka tulkitaan NaN:ksi.
/// Ilman tätä listaa polars lukisi merkkijonon "null" kirjaimellisesti,
/// jolloin `remove_null_strings_from_columns` ja `Muutostieto`-suodattimet
/// käyttäytyisivät eri tavalla kuin Python-versiossa.
pub const PANDAS_NA_VALUES: &[&str] = &[
    "", "#N/A", "#N/A N/A", "#NA", "-1.#IND", "-1.#QNAN", "-NaN", "-nan", "1.#IND", "1.#QNAN",
    "<NA>", "N/A", "NA", "NULL", "NaN", "None", "n/a", "nan", "null",
];

/// Merkkijono, joka Python-koodissa tulkitaan "ei Kela-korvattavaksi".
const EK_KORVAUSLUOKKA: &str = r#"[{"rowid":"0","korvausluokka_koodi_ja_selite":"EK","erillisselvitys_koodi_ja_selite":"null","korvattava_sairaus_koodi_ja_selite":"null"}]"#;

fn is_numeric_dtype(dt: &DataType) -> bool {
    matches!(
        dt,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64
    )
}

fn has_column(df: &DataFrame, name: &str) -> bool {
    df.get_columns().iter().any(|c| c.name().as_str() == name)
}

// ---------------------------------------------------------------------------

pub fn remove_items_which_do_not_have_the_newest_versionumero(df: DataFrame) -> PolarsResult<DataFrame> {
    df.lazy()
        .filter(col("versionumero").eq(col("versionumero").max()))
        .collect()
}

/// HUOM: Python vertaa `laakemuoto_koodi_ja_selite`-saraketta kokonaislukuihin
/// [370, 372, 373]. Jos sarake on merkkijono, pandasissa yksikään rivi ei
/// koskaan täsmää eli suodatin on käytännössä no-op. Polars heittäisi
/// tyyppivirheen, joten replikoidaan pandasin käytös ja varoitetaan.
pub fn remove_rows_which_have_non_allowed_laakemuoto(df: DataFrame) -> PolarsResult<DataFrame> {
    let dtype = df.column("laakemuoto_koodi_ja_selite")?.dtype().clone();
    if !is_numeric_dtype(&dtype) {
        eprintln!(
            "VAROITUS: sarake 'laakemuoto_koodi_ja_selite' on tyyppiä {dtype:?}, ei numeerinen. \
             Suodatinta [370, 372, 373] ei sovelleta (sama lopputulos kuin Python-versiossa)."
        );
        return Ok(df);
    }
    let banned = Series::new("banned".into(), [370i64, 372, 373]);
    df.lazy()
        .filter(
            col("laakemuoto_koodi_ja_selite")
                .is_in(lit(banned))
                .fill_null(false)
                .not(),
        )
        .collect()
}

pub fn remove_rows_where_yksilointitunnus_is_empty(df: DataFrame) -> PolarsResult<DataFrame> {
    df.drop_nulls(Some(&["yksilointitunnus".to_string()]))
}

pub fn pp_row_refining(df: DataFrame) -> PolarsResult<DataFrame> {
    let is_pp = || col("muutostieto_ltk").eq(lit("PP"));
    df.lazy()
        .with_columns([
            when(is_pp()).then(lit(0.0)).otherwise(col("tukkuhinta")).alias("tukkuhinta"),
            when(is_pp()).then(lit(0.0)).otherwise(col("verotonhinta")).alias("verotonhinta"),
            when(is_pp())
                .then(lit(0.0))
                .otherwise(col("verollinenmyyntihinta"))
                .alias("verollinenmyyntihinta"),
            when(is_pp())
                .then(lit(""))
                .otherwise(col("korvausluokka").cast(DataType::String))
                .alias("korvausluokka"),
        ])
        .collect()
}

pub fn remove_null_strings_from_columns(df: DataFrame) -> PolarsResult<DataFrame> {
    const COLUMNS_TO_CLEAN: &[&str] = &[
        "kauppanimi",
        "vahvuus",
        "laite",
        "pakkauskoko_teksti",
        "laakemuoto_fi_FI",
        "muutostieto_ltk",
        "hintamuutos_ltk",
    ];
    let exprs: Vec<Expr> = COLUMNS_TO_CLEAN
        .iter()
        .filter(|name| has_column(&df, name))
        .map(|name| {
            let as_str = col(*name).cast(DataType::String);
            when(as_str.clone().eq(lit("null")))
                .then(lit(""))
                .otherwise(as_str)
                .fill_null(lit(""))
                .alias(*name)
        })
        .collect();
    if exprs.is_empty() {
        return Ok(df);
    }
    df.lazy().with_columns(exprs).collect()
}

pub fn refine_reseptistatus(df: DataFrame) -> PolarsResult<DataFrame> {
    let dtype = df.column("reseptistatus")?.dtype().clone();
    let expr = if is_numeric_dtype(&dtype) {
        when(col("reseptistatus").eq(lit(0i64)))
            .then(lit("Ei"))
            .when(col("reseptistatus").eq(lit(1i64)))
            .then(lit("Kyllä"))
            .otherwise(col("reseptistatus").cast(DataType::String))
    } else {
        when(col("reseptistatus").eq(lit("0")))
            .then(lit("Ei"))
            .when(col("reseptistatus").eq(lit("1")))
            .then(lit("Kyllä"))
            .otherwise(col("reseptistatus").cast(DataType::String))
    };
    df.lazy().with_column(expr.alias("reseptistatus")).collect()
}

pub fn force_string_columns_to_uppercase(df: DataFrame) -> PolarsResult<DataFrame> {
    if !has_column(&df, "kauppanimi") {
        return Ok(df);
    }
    df.lazy()
        .with_column(col("kauppanimi").str().to_uppercase().alias("kauppanimi"))
        .collect()
}

/// Python: liittää sarakkeet välilyönnillä, tiivistää peräkkäiset välit
/// yhdeksi ja trimmaa reunat.
pub fn create_pitka_tuotenimi_column(df: DataFrame) -> PolarsResult<DataFrame> {
    let parts = [
        col("kauppanimi"),
        col("laakemuoto_fi_FI"),
        col("vahvuus"),
        col("laite"),
        col("pakkauskoko_teksti"),
    ];
    df.lazy()
        .with_column(
            concat_str(parts, " ", true)
                .str()
                .replace_all(lit(r"\s+"), lit(" "), false)
                .str()
                .strip_chars(lit(Null {}))
                .alias("pitka_tuotenimi"),
        )
        .collect()
}

pub fn sort_dataframe_based_on_columns(df: DataFrame) -> PolarsResult<DataFrame> {
    df.lazy()
        .sort_by_exprs(
            [col("reseptistatus"), col("pitka_tuotenimi")],
            SortMultipleOptions::default()
                .with_order_descending_multi([false, false])
                .with_nulls_last(true),
        )
        .collect()
}

pub fn cut_column_string_length(df: DataFrame, column: &str, length: u64) -> PolarsResult<DataFrame> {
    if !has_column(&df, column) {
        return Ok(df);
    }
    df.lazy()
        .with_column(
            col(column)
                .cast(DataType::String)
                .str()
                .slice(lit(0i64), lit(length))
                .alias(column),
        )
        .collect()
}

pub fn round_float_column(df: DataFrame, column: &str, precision: u32) -> PolarsResult<DataFrame> {
    if !has_column(&df, column) {
        return Err(PolarsError::ColumnNotFound(
            format!("Column '{column}' not found in DataFrame.").into(),
        ));
    }
    df.lazy().with_column(col(column).round(precision).alias(column)).collect()
}

pub fn add_tukkuhinta_erotus_columns(df: DataFrame) -> PolarsResult<DataFrame> {
    df.lazy()
        .with_column((col("tukkuhinta") - col("tukkuhinta_edellinen_taksa")).alias("tukkuhinta_erotus_eur"))
        .with_column(
            (col("tukkuhinta_erotus_eur") / col("tukkuhinta_edellinen_taksa") * lit(100.0))
                .alias("tukkuhinta_erotus_prosentti"),
        )
        .collect()
}

/// Purkaa `toimittajat`-JSONin ja mappaa toimittajakoodit tukkunimiksi.
/// Python tuottaa Python-listan repr:n ilman hakasulkeita ja heittomerkkejä,
/// eli "a, b, c". Virheellinen tai puuttuva JSON -> tyhjä merkkijono.
pub fn assign_tukku(mut df: DataFrame) -> PolarsResult<DataFrame> {
    let tukku_map: HashMap<&str, &str> = HashMap::from([
        ("0", "circlumfarmasia"),
        ("1", "magnum"),
        ("2", "vitabalans"),
        ("3", "medifon"),
        ("4", "suppilog"),
        ("5", "tenhunen"),
        ("6", "toysanapteekki"),
        ("7", "oriola"),
        ("8", "tamro"),
        ("9", "yliopistonapteekki"),
        ("10", "repolar"),
    ]);

    let series = df.column("toimittajat")?.cast(&DataType::String)?;
    let chunked = series.as_materialized_series().str()?;

    let values: Vec<String> = chunked
        .into_iter()
        .map(|opt| match opt {
            Some(json) => extract_tukku_list(json, &tukku_map),
            None => String::new(),
        })
        .collect();

    df.with_column(Series::new("Tukku".into(), values))?;
    Ok(df)
}

fn extract_tukku_list(json: &str, tukku_map: &HashMap<&str, &str>) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    let Some(items) = parsed.as_array() else {
        return String::new();
    };
    items
        .iter()
        .filter_map(|item| item.get("toimittajat_toimittaja"))
        .filter_map(|code| code.as_str())
        .filter_map(|code| tukku_map.get(code).copied())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn add_kelakorvattava_column(df: DataFrame) -> PolarsResult<DataFrame> {
    df.lazy()
        .with_column(
            when(
                col("korvausluokka")
                    .eq(lit(""))
                    .or(col("korvausluokka").eq(lit(EK_KORVAUSLUOKKA))),
            )
            .then(lit("Ei"))
            .otherwise(lit("Kyllä"))
            .alias("Kelakorvattava"),
        )
        .collect()
}

pub fn refine_apteekkiveroperuste_column(df: DataFrame) -> PolarsResult<DataFrame> {
    if !has_column(&df, "Apteekkiveroperuste") {
        return Err(PolarsError::ColumnNotFound(
            "Column 'Apteekkiveroperuste' not found in the DataFrame.".into(),
        ));
    }
    let dtype = df.column("Apteekkiveroperuste")?.dtype().clone();
    let expr = if is_numeric_dtype(&dtype) {
        when(col("Apteekkiveroperuste").eq(lit(1i64)))
            .then(lit("Veron alainen"))
            .when(col("Apteekkiveroperuste").eq(lit(0i64)))
            .then(lit("Veron ulkopuolinen"))
            .otherwise(col("Apteekkiveroperuste").cast(DataType::String))
    } else {
        when(col("Apteekkiveroperuste").eq(lit("1")))
            .then(lit("Veron alainen"))
            .when(col("Apteekkiveroperuste").eq(lit("0")))
            .then(lit("Veron ulkopuolinen"))
            .otherwise(col("Apteekkiveroperuste").cast(DataType::String))
    };
    df.lazy()
        .with_column(expr.alias("Apteekkiveroperuste"))
        .collect()
}

pub fn drop_unused_columns(df: DataFrame) -> DataFrame {
    df.drop_many([
        "toimittajat",
        "kauppanimi",
        "vahvuus",
        "laite",
        "modified_external_kela_integration",
        "laakemuoto_fi_FI",
        "versionumero",
        "korvausluokka",
    ])
}

pub fn rename_columns_for_final_report(df: DataFrame) -> PolarsResult<DataFrame> {
    const RENAMES: &[(&str, &str)] = &[
        ("yksilointitunnus", "VNR"),
        ("pitka_tuotenimi", "Pitkä tuotenimi"),
        ("tukkuhinta", "THsALV"),
        ("verotonhinta", "VHsALV"),
        ("verollinenmyyntihinta", "VHcALV"),
        ("pakkauskoko_teksti", "Pakkauskoko"),
        ("reseptistatus", "Reseptivalmiste"),
        ("tukkuhinta_erotus_prosentti", "Tukkuhinta erotus (%)"),
        ("tukkuhinta_erotus_eur", "Tukkuhinta erotus (eur)"),
        ("myyntiluvan_haltija", "Myyntiluvan haltija"),
        ("muutostieto_ltk", "Muutostieto"),
        ("hintamuutos_ltk", "Hintamuutos"),
        ("tukkuhinta_edellinen_taksa", "THsALV (ed.taksa)"),
        ("apteekkiveroperuste", "Apteekkiveroperuste"),
    ];
    let existing: Vec<&str> = RENAMES.iter().map(|(old, _)| *old).collect();
    let new: Vec<&str> = RENAMES.iter().map(|(_, new)| *new).collect();
    df.lazy().rename(existing, new, true).collect()
}
