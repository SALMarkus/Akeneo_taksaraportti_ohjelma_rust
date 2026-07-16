//! XLSX-vienti. Vastaa Python-version `write_datapandas_to_file`-funktiota
//! (`df.to_excel(path, index=False, engine='openpyxl')`).

use polars::prelude::*;
use rust_xlsxwriter::Workbook;

pub fn write_datapandas_to_file(df: &DataFrame, export_path: &str) {
    match write_workbook(df, export_path) {
        Ok(()) => println!("File successfully exported to {export_path}"),
        Err(e) => println!("An error occurred while exporting the file: {e}"),
    }
}

fn write_workbook(df: &DataFrame, export_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();

    for (col_idx, column) in df.get_columns().iter().enumerate() {
        let col_idx = col_idx as u16;
        worksheet.write_string(0, col_idx, column.name().as_str())?;

        let series = column.as_materialized_series();
        for row_idx in 0..series.len() {
            let row = row_idx as u32 + 1;
            match series.get(row_idx)? {
                // pandas kirjoittaa NaN:n tyhjänä soluna.
                AnyValue::Null => {}
                AnyValue::String(v) => {
                    worksheet.write_string(row, col_idx, v)?;
                }
                AnyValue::StringOwned(v) => {
                    worksheet.write_string(row, col_idx, v.as_str())?;
                }
                AnyValue::Boolean(v) => {
                    worksheet.write_boolean(row, col_idx, v)?;
                }
                other => match other.try_extract::<f64>() {
                    // NaN/inf syntyy nollalla jaosta tukkuhinta_erotus_prosentti-sarakkeessa.
                    // Excelissä ei ole näille esitystä, joten solu jätetään tyhjäksi.
                    Ok(n) if n.is_nan() || n.is_infinite() => {}
                    Ok(n) => {
                        worksheet.write_number(row, col_idx, n)?;
                    }
                    Err(_) => {
                        worksheet.write_string(row, col_idx, &other.to_string())?;
                    }
                },
            }
        }
    }

    workbook.save(export_path)?;
    Ok(())
}
