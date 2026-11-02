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
//! three ways. Photos (images), notes (text) and what the trip cost (a
//! table). The owner removes them like any other Space.
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

    vec![
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
    ]
}
