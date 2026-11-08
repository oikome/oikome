// Copyright 2025 The Oikome Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The examples a new data center starts with, so there is something to
//! look at, open and share on the first day, one of each kind: photos of a
//! weekend in Lisbon, a short guide to Oikome itself (notes), three weeks of
//! spending that end with that weekend (a table, as a bank exports it), and
//! two weeks of a home's temperatures, hour by hour (readings).
//!
//! They are files, written once into the `Examples` folder of where the
//! owner keeps their data, like a notes app's vault: photos with a
//! `captions.csv`, notes as Markdown, tables as CSV. Each folder is a
//! source (§11) of the collection of its name, read again every minute, so
//! the owner can edit the files with any app. A file already there is left
//! as it is; the owner removes an example like any other collection, and
//! its files stay.
//!
//! Every value here is a constant of this file.

/// How an example's folder is read.
pub(crate) enum Feed {
    Photos,
    Notes,
    /// One CSV file of the folder, as the table `table`.
    Csv {
        file: &'static str,
        table: &'static str,
    },
}

/// One example: the collection's name, the files of its folder, and how
/// they are read.
pub(crate) struct Example {
    pub name: &'static str,
    pub files: Vec<(String, Vec<u8>)>,
    pub feed: Feed,
    /// The time to give the files, a minute apart in their order, for those
    /// that say no time of their own (Unix seconds); `None` leaves now.
    pub modified: Option<u64>,
}

/// The guide's notes are dated the Sunday before the trip, 9:00 UTC, so
/// what the owner edits later comes first.
const GUIDE_WRITTEN: u64 = 1_761_469_200;

/// A photo of the trip. The pictures are from Wikimedia Commons; `credit`
/// names each one's author and licence, and travels with the row.
struct Photo {
    title: &'static str,
    place: &'static str,
    taken: &'static str,
    jpeg: &'static [u8],
    credit: &'static str,
}

const PHOTOS: &[Photo] = &[
    Photo {
        title: "Alfama from Santa Luzia",
        place: "Alfama",
        taken: "2025-11-07 09:40:00",
        jpeg: include_bytes!("../assets/lisbon/alfama.jpg"),
        credit: "\"Lisbon rooftops\" by Naval S from Berlin, CC BY 2.0, via Wikimedia Commons",
    },
    Photo {
        title: "Pastéis de Belém",
        place: "Belém",
        taken: "2025-11-07 11:05:00",
        jpeg: include_bytes!("../assets/lisbon/pasteis.jpg"),
        credit: "\"Pasteis de Belem\" by Heather Cowper from Bristol, UK, CC BY 2.0, via Wikimedia Commons",
    },
    Photo {
        title: "Belém Tower",
        place: "Belém",
        taken: "2025-11-07 12:30:00",
        jpeg: include_bytes!("../assets/lisbon/belem-tower.jpg"),
        credit: "\"Belem Tower\" by Neil, CC BY 2.0, via Wikimedia Commons",
    },
    Photo {
        title: "Tram 28E",
        place: "Graça",
        taken: "2025-11-08 10:15:00",
        jpeg: include_bytes!("../assets/lisbon/tram.jpg"),
        credit: "\"Tram of line 28E on Rua Augusto Rosa in Lisbon\" by Jakub Hałun, CC BY 4.0, via Wikimedia Commons",
    },
    Photo {
        title: "Tiled houses",
        place: "Lisbon",
        taken: "2025-11-08 15:20:00",
        jpeg: include_bytes!("../assets/lisbon/tiles.jpg"),
        credit: "\"Azulejos in Lisboa\" by Nan Palmero from San Antonio, TX, USA, CC BY 2.0, via Wikimedia Commons",
    },
    Photo {
        title: "Sunset on the Tagus",
        place: "Riverside",
        taken: "2025-11-09 18:05:00",
        jpeg: include_bytes!("../assets/lisbon/sunset.jpg"),
        credit: "\"Sunset in Lisbon\", CC0, via Wikimedia Commons",
    },
];

/// Day, what, category, euros.
const SPENDING: &[(&str, &str, &str, &str)] = &[
    ("2025-10-20", "Groceries for the week", "Food", "46.20"),
    ("2025-10-20", "Monthly transit pass", "Transport", "40.00"),
    ("2025-10-21", "Coffee with Ana", "Food", "6.40"),
    ("2025-10-22", "Haircut", "Personal", "18.00"),
    ("2025-10-23", "Lunch near the office", "Food", "11.50"),
    ("2025-10-24", "Cinema tickets", "Going out", "19.00"),
    ("2025-10-25", "Farmers' market", "Food", "23.80"),
    (
        "2025-10-26",
        "Train to see my parents",
        "Transport",
        "24.60",
    ),
    ("2025-10-27", "Phone bill", "Bills", "15.00"),
    ("2025-10-28", "Groceries after work", "Food", "31.75"),
    ("2025-10-29", "A present for Mum", "Shopping", "35.00"),
    ("2025-10-30", "A guidebook to Lisbon", "Shopping", "16.90"),
    ("2025-10-31", "Dinner with friends", "Going out", "42.00"),
    ("2025-11-01", "Electricity bill", "Bills", "58.30"),
    ("2025-11-02", "Sunday brunch", "Food", "21.40"),
    (
        "2025-11-03",
        "Groceries for the week ahead",
        "Food",
        "52.10",
    ),
    ("2025-11-04", "Bike repair", "Transport", "28.00"),
    ("2025-11-05", "Flight to Lisbon", "Transport", "89.00"),
    ("2025-11-06", "Travel adapter", "Shopping", "12.99"),
    ("2025-11-07", "Pastéis de Belém and coffee", "Food", "7.80"),
    ("2025-11-07", "24-hour transport pass", "Transport", "6.80"),
    ("2025-11-07", "Belém Tower tickets", "Sights", "16.00"),
    ("2025-11-07", "Dinner in Rua das Flores", "Food", "38.50"),
    ("2025-11-08", "Lunch at Time Out Market", "Food", "21.00"),
    (
        "2025-11-08",
        "Pessoa's poems at Ler Devagar",
        "Shopping",
        "24.90",
    ),
    ("2025-11-08", "Ginjinha in Rossio", "Going out", "3.00"),
    (
        "2025-11-09",
        "A tile from Feira da Ladra",
        "Shopping",
        "15.00",
    ),
    ("2025-11-09", "Fado at Tasca do Chico", "Going out", "27.00"),
    ("2025-11-09", "Metro to the airport", "Transport", "1.85"),
];

/// What Oikome is and how to use it, a note at a time, in reading order.
/// The UI's own words: collection, contact, share, Activity, recovery key.
const GUIDE: &[&str] = &[
    "Welcome to Oikome. It keeps your photos, notes and tables on your own computer, in \
     collections, and nowhere else unless you share them.",
    "Everything you see now is an example, kept as plain files in the Examples folder where \
     your data lives. Edit a note, a CSV file or a photo's caption with any app, and Oikome \
     reads the change within a minute. Photos and notes are read where they are; Sources \
     shows where each table comes from.",
    "New Collection makes one of your own. Choose where it is stored: Local, a \
     folder, a NAS or a cloud drive. Settings \u{2192} Storage picks where new ones go.",
    "To share a collection, add the person as a contact with their ID, then press Share. \
     You choose whether they view it live or keep a copy, and until when; they get an invite \
     code to open it.",
    "Stop Sharing takes effect at once: their next read is turned away. Activity shows every \
     read and every refusal, and it is kept only on this computer.",
    "Back up your recovery key. It is your identity: with it, this data center can be brought \
     back on a new computer, and no one else has a copy.",
    "Press \u{2318}K (Ctrl+K on Windows and Linux) to jump to a collection, a person or a page, \
     or to switch between the Ink and Bold styles.",
];

/// Hours of temperature and humidity readings: two weeks.
const CLIMATE_HOURS: u32 = 14 * 24;

/// A steady wobble in -1..1 that changes over `period` hours: noise that
/// looks measured, the same on every computer.
fn wobble(hour: f64, period: f64, seed: u64) -> f64 {
    let at = |k: u64| {
        let mut x = (k ^ seed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 31;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 29;
        (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    let t = hour / period;
    let k = t.floor();
    let f = t - k;
    let f = f * f * (3.0 - 2.0 * f);
    at(k as u64) * (1.0 - f) + at(k as u64 + 1) * f
}

/// A bump around `center` o'clock, `width` hours wide.
fn around(hour_of_day: f64, center: f64, width: f64) -> f64 {
    let d = (hour_of_day - center + 12.0).rem_euclid(24.0) - 12.0;
    (-d * d / (2.0 * width * width)).exp()
}

/// One hour's readings at home, made up but shaped like the real thing: the
/// heating comes on in the morning and the evening, the weekend keeps the
/// living room warm at midday, the bedroom stays cooler, and outside follows
/// the sun and a week's weather. Starts on a Monday.
fn climate(hour: u32) -> (f64, f64, f64, i64) {
    use std::f64::consts::TAU;
    let i = f64::from(hour);
    let h = i % 24.0;
    let weekend = matches!((hour / 24) % 7, 5 | 6);
    let outside = 10.5
        + 3.8 * (TAU * (h - 9.0) / 24.0).sin()
        + 3.0 * (TAU * i / (24.0 * 7.0)).sin()
        + 1.2 * wobble(i, 5.0, 1);
    let living = 19.4
        + 1.1 * around(h, 7.5, 1.1)
        + 2.4 * around(h, 19.5, 2.2)
        + if weekend { around(h, 13.0, 3.0) } else { 0.0 }
        + 0.08 * (outside - 10.5)
        + 0.3 * wobble(i, 3.0, 2);
    let bedroom = 18.3
        + 0.6 * around(h, 21.0, 2.0)
        + 0.5 * around(h, 3.0, 3.0)
        + 0.06 * (outside - 10.5)
        + 0.25 * wobble(i, 4.0, 3);
    let humidity = (54.0 - 1.3 * (outside - 10.5) + 3.0 * wobble(i, 6.0, 4)).clamp(35.0, 75.0);
    (living, bedroom, outside, humidity.round() as i64)
}

/// One CSV line, each field quoted when it needs to be.
fn csv_line(fields: &[String]) -> String {
    let quoted: Vec<String> = fields
        .iter()
        .map(|f| {
            if f.contains([',', '"', '\n']) {
                format!("\"{}\"", f.replace('"', "\"\""))
            } else {
                f.clone()
            }
        })
        .collect();
    quoted.join(",") + "\n"
}

fn csv(header: &[&str], rows: impl IntoIterator<Item = Vec<String>>) -> Vec<u8> {
    let header: Vec<String> = header.iter().map(|h| h.to_string()).collect();
    let mut out = csv_line(&header);
    for row in rows {
        out.push_str(&csv_line(&row));
    }
    out.into_bytes()
}

/// The examples, in the order they are created.
pub(crate) fn examples() -> Vec<Example> {
    let guide_titles = [
        "Welcome",
        "The examples",
        "New collections",
        "Sharing",
        "Stopping and Activity",
        "Recovery key",
        "Shortcuts",
    ];
    let guide = GUIDE
        .iter()
        .zip(guide_titles)
        .enumerate()
        .map(|(i, (text, title))| {
            (
                format!("{} {title}.md", i + 1),
                format!("{text}\n").into_bytes(),
            )
        })
        .collect();

    let mut photos: Vec<(String, Vec<u8>)> = PHOTOS
        .iter()
        .map(|p| (format!("{}.jpg", p.title), p.jpeg.to_vec()))
        .collect();
    photos.push((
        "captions.csv".to_string(),
        csv(
            &["file", "title", "place", "taken", "credit"],
            PHOTOS.iter().map(|p| {
                vec![
                    format!("{}.jpg", p.title),
                    p.title.to_string(),
                    p.place.to_string(),
                    p.taken[..16].to_string(),
                    p.credit.to_string(),
                ]
            }),
        ),
    ));

    let spending = vec![(
        "spending.csv".to_string(),
        csv(
            &["what", "day", "category", "euros"],
            SPENDING.iter().map(|(day, what, category, euros)| {
                vec![
                    what.to_string(),
                    day.to_string(),
                    category.to_string(),
                    euros.to_string(),
                ]
            }),
        ),
    )];

    let readings = vec![(
        "readings.csv".to_string(),
        csv(
            &[
                "at",
                "living_room_celsius",
                "bedroom_celsius",
                "outside_celsius",
                "humidity_pct",
            ],
            (0..CLIMATE_HOURS).map(|hour| {
                let (living, bedroom, outside, humidity) = climate(hour);
                // From midnight on Monday, October 27, 2025, to the trip's last evening.
                let day = 27 + hour / 24;
                let (month, day) = if day > 31 { (11, day - 31) } else { (10, day) };
                vec![
                    format!("2025-{month:02}-{day:02} {:02}:00:00", hour % 24),
                    format!("{living:.1}"),
                    format!("{bedroom:.1}"),
                    format!("{outside:.1}"),
                    humidity.to_string(),
                ]
            }),
        ),
    )];

    vec![
        Example {
            name: "oikome-guide",
            files: guide,
            feed: Feed::Notes,
            modified: Some(GUIDE_WRITTEN),
        },
        Example {
            name: "lisbon-photos",
            files: photos,
            feed: Feed::Photos,
            modified: None,
        },
        Example {
            name: "spending",
            files: spending,
            feed: Feed::Csv {
                file: "spending.csv",
                table: "spending",
            },
            modified: None,
        },
        Example {
            name: "home-climate",
            files: readings,
            feed: Feed::Csv {
                file: "readings.csv",
                table: "readings",
            },
            modified: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_fields_are_quoted_when_they_need_it() {
        assert_eq!(
            csv_line(&["a".into(), "b, c".into(), "say \"hi\"".into()]),
            "a,\"b, c\",\"say \"\"hi\"\"\"\n"
        );
    }
}
