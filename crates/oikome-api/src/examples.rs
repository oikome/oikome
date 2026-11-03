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
//! look at, open and share on the first day: one weekend in Lisbon, kept
//! three ways (photos, notes and what the trip cost), a short guide to
//! Oikome itself, a reading list, and two weeks of a home's temperatures,
//! hour by hour. The owner removes them like any other Space.
//!
//! Every value here is a constant of this file; the statements quote them
//! like any literal and take nothing from a request.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use oikome_semantic::rewrite::quote_ident;

/// One example Space and the statements that fill it, after it is created.
pub(crate) struct Example {
    pub name: &'static str,
    pub statements: Vec<String>,
}

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

const NOTES: &[(&str, &str)] = &[
    (
        "2025-11-07 08:15:00",
        "Pastéis de Belém opens at 8. There was already a line at 8:15, but it moves fast. \
         Eat them warm, with cinnamon and sugar on top.",
    ),
    (
        "2025-11-07 12:50:00",
        "The Belém Tower queue is shorter after lunch. The walk along the river from the \
         monastery takes ten minutes.",
    ),
    (
        "2025-11-07 19:30:00",
        "Dinner at the counter in Rua das Flores. No bookings: put your name down and wait \
         with a glass of wine across the street.",
    ),
    (
        "2025-11-08 09:45:00",
        "Tram 28 is full by ten. Walk up to Graça instead and ride it down through Alfama.",
    ),
    (
        "2025-11-08 13:10:00",
        "Time Out Market: skip the lobster, get a bifana at the counter by the door.",
    ),
    (
        "2025-11-08 16:40:00",
        "Ler Devagar, in LX Factory, has a bicycle flying over the shelves. Bought a small \
         book of Pessoa's poems.",
    ),
    (
        "2025-11-09 20:30:00",
        "Fado in Alfama: book Tasca do Chico ahead, or come before nine.",
    ),
];

/// Day, what, category, euros.
const SPENDING: &[(&str, &str, &str, &str)] = &[
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
    "Everything you see now is an example. Open a collection to see it as photos, notes, \
     a table or readings over time; remove an example from its More menu when you are done \
     with it.",
    "New Collection makes one of your own. Choose where it is stored: this computer, a \
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

/// Title, author, status, pages, finished (when read).
const READING: &[(&str, &str, &str, u32, Option<&str>)] = &[
    (
        "The Book of Disquiet",
        "Fernando Pessoa",
        "Reading",
        544,
        None,
    ),
    (
        "Journey to Portugal",
        "José Saramago",
        "Read",
        464,
        Some("2025-11-05"),
    ),
    (
        "Pereira Maintains",
        "Antonio Tabucchi",
        "Read",
        192,
        Some("2025-10-18"),
    ),
    (
        "Blindness",
        "José Saramago",
        "Read",
        326,
        Some("2025-09-27"),
    ),
    (
        "Night Train to Lisbon",
        "Pascal Mercier",
        "Read",
        438,
        Some("2025-08-30"),
    ),
    (
        "The Return",
        "Dulce Maria Cardoso",
        "Read",
        256,
        Some("2025-07-14"),
    ),
    (
        "The Year of the Death of Ricardo Reis",
        "José Saramago",
        "Next",
        358,
        None,
    ),
    (
        "Requiem: A Hallucination",
        "Antonio Tabucchi",
        "Next",
        112,
        None,
    ),
    ("The Maias", "Eça de Queirós", "Next", 720, None),
    ("The Lusiads", "Luís Vaz de Camões", "Next", 288, None),
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

/// A SQL string literal of a constant above.
fn text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn table(space: &str, table: &str) -> String {
    format!("{}.{}", quote_ident(space), quote_ident(table))
}

/// The examples, in the order they are created.
pub(crate) fn examples() -> Vec<Example> {
    let photos = table("lisbon-photos", "photos");
    let mut photo_statements = vec![format!(
        "CREATE TABLE {photos} (title VARCHAR, place VARCHAR, taken TIMESTAMP, photo BLOB, credit VARCHAR)"
    )];
    photo_statements.extend(PHOTOS.iter().map(|p| {
        format!(
            "INSERT INTO {photos} VALUES ({}, {}, TIMESTAMP {}, from_base64({}), {})",
            text(p.title),
            text(p.place),
            text(p.taken),
            text(&STANDARD.encode(p.jpeg)),
            text(p.credit),
        )
    }));

    let notes = table("lisbon-notes", "notes");
    let note_rows: Vec<String> = NOTES
        .iter()
        .map(|(written, note)| format!("(TIMESTAMP {}, {})", text(written), text(note)))
        .collect();

    let spending = table("lisbon-spending", "spending");
    let spending_rows: Vec<String> = SPENDING
        .iter()
        .map(|(day, what, category, euros)| {
            format!(
                "(DATE {}, {}, {}, {euros})",
                text(day),
                text(what),
                text(category)
            )
        })
        .collect();

    let guide = table("oikome-guide", "guide");
    let guide_rows: Vec<String> = GUIDE
        .iter()
        .map(|note| format!("({})", text(note)))
        .collect();

    let reading = table("reading-list", "books");
    let reading_rows: Vec<String> = READING
        .iter()
        .map(|(title, author, status, pages, finished)| {
            let finished = finished.map_or("NULL".to_string(), |day| format!("DATE {}", text(day)));
            format!(
                "({}, {}, {}, {pages}, {finished})",
                text(title),
                text(author),
                text(status)
            )
        })
        .collect();

    let readings = table("home-climate", "readings");
    let climate_rows: Vec<String> = (0..CLIMATE_HOURS)
        .map(|hour| {
            let (living, bedroom, outside, humidity) = climate(hour);
            // From midnight on Monday, October 27, 2025, to the trip's last evening.
            let day = 27 + hour / 24;
            let (month, day) = if day > 31 { (11, day - 31) } else { (10, day) };
            format!(
                "(TIMESTAMP '2025-{month:02}-{day:02} {:02}:00:00', {living:.1}, {bedroom:.1}, {outside:.1}, {humidity})",
                hour % 24
            )
        })
        .collect();

    vec![
        Example {
            name: "oikome-guide",
            statements: vec![
                format!("CREATE TABLE {guide} (note VARCHAR)"),
                format!("INSERT INTO {guide} VALUES {}", guide_rows.join(", ")),
            ],
        },
        Example {
            name: "lisbon-photos",
            statements: photo_statements,
        },
        Example {
            name: "lisbon-notes",
            statements: vec![
                format!("CREATE TABLE {notes} (written TIMESTAMP, note VARCHAR)"),
                format!("INSERT INTO {notes} VALUES {}", note_rows.join(", ")),
            ],
        },
        Example {
            name: "lisbon-spending",
            statements: vec![
                format!(
                    "CREATE TABLE {spending} (day DATE, what VARCHAR, category VARCHAR, euros DECIMAL(8, 2))"
                ),
                format!("INSERT INTO {spending} VALUES {}", spending_rows.join(", ")),
            ],
        },
        Example {
            name: "reading-list",
            statements: vec![
                format!(
                    "CREATE TABLE {reading} (title VARCHAR, author VARCHAR, status VARCHAR, pages INTEGER, finished DATE)"
                ),
                format!("INSERT INTO {reading} VALUES {}", reading_rows.join(", ")),
            ],
        },
        Example {
            name: "home-climate",
            statements: vec![
                format!(
                    "CREATE TABLE {readings} (at TIMESTAMP, living_room_celsius DECIMAL(4, 1), bedroom_celsius DECIMAL(4, 1), outside_celsius DECIMAL(4, 1), humidity_pct INTEGER)"
                ),
                format!("INSERT INTO {readings} VALUES {}", climate_rows.join(", ")),
            ],
        },
    ]
}
