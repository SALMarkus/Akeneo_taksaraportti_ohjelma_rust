# Akeneo Taksaraportti -ohjelma (Rust)

Käännös alkuperäisestä `Akeneo_Taksaraportti_ohjelma.py`-skriptistä.

## Kääntäminen

```
cargo build --release
```

Binääri: `target/release/akeneo_taksaraportti_ohjelma.exe`

Riippuvuudet on **pinnattu tarkoituksella** (`Cargo.toml`). Polarsin API muuttuu
julkaisujen välillä ja printpdf 0.8 rikkoi 0.7:n API:n. Älä päivitä versioita
ilman että käyt koodin läpi.

## Rakenne

| Tiedosto | Vastuu |
|---|---|
| `src/main.rs` | Kansiot, CSV:n luku, muunnosketju, viisi raporttia |
| `src/transform.rs` | Datamuunnokset (vastaa Pythonin muunnosfunktioita) |
| `src/excel.rs` | XLSX-vienti (`to_excel`-vastine) |
| `src/pdf.rs` | PDF-taulukot (`reportlab`-vastine, käsin piirretty) |
| `src/dates.rs` | Taksan voimaantulopäivän laskenta |

## Tiedossa olevat erot Python-versioon

1. **PDF ei ole pikselintarkka.** reportlabin `Table` on toteutettu käsin.
   Sarakeleveydet, rivikorkeudet, fonttikoot ja sivutus vastaavat Pythonia,
   mutta pystykeskitys (`VALIGN=MIDDLE` + `BOTTOMPADDING=2`) on approksimaatio.

2. **Pyöristys.** pandas `.round(2)` käyttää pankkiirin pyöristystä
   (puolikkaat lähimpään parilliseen), polars pyöristää puolikkaat nollasta
   poispäin. Ero näkyy vain arvoilla, jotka osuvat tasan .005:een.

3. **NaN/inf Excelissä.** `tukkuhinta_erotus_prosentti` voi olla NaN tai inf
   (jako nollalla). Nämä kirjoitetaan tyhjänä soluna.

4. **`laakemuoto_koodi_ja_selite`-suodatin.** Python vertaa merkkijonosaraketta
   kokonaislukuihin `[370, 372, 373]`, jolloin suodatin ei koskaan osu.
   Rust-versio replikoi tämän ja tulostaa varoituksen. Jos suodattimen on
   tarkoitus toimia, tämä pitää korjata erikseen.

## Pythonista löytyneet asiat, joita ei korjattu

- `tukkuhinta_erotus_prosentti` jakaa nollalla ilman tarkistusta.
- Sivutuksessa ensimmäisellä sivulla on yksi datarivi vähemmän kuin muilla,
  koska otsikkorivi lasketaan mukaan `max_rows_per_page`-rajaan.
- Hintamuutoslistan sarakeleveyksien summa on 289 mm, mikä ylittää A4-vaaka-
  arkin käytettävissä olevan 282,8 mm:n leveyden.
- `current_timestamp_in_string_format_for_filename` ei ole käytössä.
