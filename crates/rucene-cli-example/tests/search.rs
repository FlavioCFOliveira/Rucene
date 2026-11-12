//! `search`: every field kind alone, combined criteria, and sorting, each
//! compared with the result computed independently from the fixture rows.

mod common;

use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::OnceLock;

use common::{
    fixture_index, fixture_rows, haversine_meters, ids, missing_last, run, run_err, search_ids,
    sorted, Row, Tsv,
};

/// One index of the whole fixture, shared by every test of this binary
/// (searches never write).
fn index() -> &'static PathBuf {
    static INDEX: OnceLock<PathBuf> = OnceLock::new();
    INDEX.get_or_init(|| fixture_index("search"))
}

fn rows() -> &'static [Row] {
    static ROWS: OnceLock<Vec<Row>> = OnceLock::new();
    ROWS.get_or_init(fixture_rows)
}

fn matching(filter: impl Fn(&Row) -> bool) -> Vec<&'static Row> {
    rows().iter().filter(|row| filter(row)).collect()
}

/// Asserts that `args` matches exactly the rows `filter` keeps (as a set),
/// and that the set is neither empty nor everything.
fn assert_set(args: &[&str], filter: impl Fn(&Row) -> bool) {
    let expected = matching(filter);
    assert!(
        !expected.is_empty() && expected.len() < rows().len(),
        "{args:?}: the expectation must be selective (got {} of {})",
        expected.len(),
        rows().len()
    );
    let (hits, total) = search_ids(index(), args);
    assert_eq!(total, expected.len() as i64, "{args:?}: total hits");
    assert_eq!(sorted(hits), sorted(ids(&expected)), "{args:?}: hit set");
}

/// Asserts that `args` (with a sort) returns exactly `expected`, in order.
fn assert_order(args: &[&str], expected: &[&Row]) {
    let (hits, total) = search_ids(index(), args);
    assert_eq!(total, expected.len() as i64, "{args:?}: total hits");
    assert_eq!(hits, ids(expected), "{args:?}: hit order");
}

fn in_range(value: Option<f64>, low: f64, high: f64) -> bool {
    value.is_some_and(|value| (low..=high).contains(&value))
}

// ---------------------------------------------------------------- criteria

#[test]
fn text_requires_every_analysed_word() {
    assert_set(&["--text", "Japan"], |row| {
        row.place_words().contains(&"japan".to_owned())
    });
    assert_set(&["--text", "port vanuatu"], |row| {
        let words = row.place_words();
        words.contains(&"port".to_owned()) && words.contains(&"vanuatu".to_owned())
    });
}

#[test]
fn phrase_requires_adjacent_words() {
    assert_set(&["--phrase", "Puerto Rico"], |row| {
        row.place_words()
            .windows(2)
            .any(|pair| pair[0] == "puerto" && pair[1] == "rico")
    });
    // "of anamizu" is a phrase; "anamizu of" never occurs.
    assert_set(&["--phrase", "of Anamizu"], |row| {
        row.place_words()
            .windows(2)
            .any(|pair| pair[0] == "of" && pair[1] == "anamizu")
    });
    let (hits, total) = search_ids(index(), &["--phrase", "anamizu of"]);
    assert_eq!((hits.len(), total), (0, 0));
}

#[test]
fn keyword_term_and_term_set() {
    assert_set(&["--term", "magType=ml"], |row| row.raw("magType") == "ml");
    assert_set(&["--terms", "net=pr,tx,hv"], |row| {
        ["pr", "tx", "hv"].contains(&row.raw("net"))
    });
    // The exact copy of the text field.
    let place = rows()[0].raw("place");
    assert_set(&["--term", &format!("place={place}")], |row| {
        row.raw("place") == place
    });
}

#[test]
fn id_lookup() {
    let target = &rows()[42];
    let (hits, total) = search_ids(index(), &["--id", target.id()]);
    assert_eq!((hits, total), (vec![target.id().to_owned()], 1));
}

#[test]
fn double_ranges_closed_and_open() {
    assert_set(&["--range", "mag=4.5..6"], |row| {
        in_range(row.f64("mag"), 4.5, 6.0)
    });
    assert_set(&["--range", "mag=5.."], |row| {
        in_range(row.f64("mag"), 5.0, f64::INFINITY)
    });
    assert_set(&["--range", "depth=..10"], |row| {
        in_range(row.f64("depth"), f64::NEG_INFINITY, 10.0)
    });
    // A column with empty values: those rows never match.
    assert_set(&["--range", "magError=0..0.05"], |row| {
        in_range(row.f64("magError"), 0.0, 0.05)
    });
}

#[test]
fn double_exact_and_set() {
    assert_set(&["--term", "mag=4.5"], |row| row.f64("mag") == Some(4.5));
    assert_set(&["--terms", "mag=4.5,5"], |row| {
        row.f64("mag") == Some(4.5) || row.f64("mag") == Some(5.0)
    });
}

#[test]
fn int_range_and_exact() {
    assert_set(&["--range", "nst=50..100"], |row| {
        row.i64("nst").is_some_and(|nst| (50..=100).contains(&nst))
    });
    assert_set(&["--range", "magNst=..10"], |row| {
        row.i64("magNst").is_some_and(|n| n <= 10)
    });
    let nst = rows()
        .iter()
        .find_map(|row| row.i64("nst"))
        .expect("an nst value");
    assert_set(&["--term", &format!("nst={nst}")], |row| {
        row.i64("nst") == Some(nst)
    });
}

#[test]
fn date_ranges_on_time_and_updated() {
    // The fixture's dates are fixed-width UTC ISO strings, so string order is
    // time order: the expectation needs no date arithmetic.
    let low = "2024-01-02T00:00:00.000Z";
    let high = "2024-01-02T12:00:00.000Z";
    assert_set(&["--range", "time=2024-01-02..2024-01-02T12:00"], |row| {
        (low..=high).contains(&row.raw("time"))
    });
    assert_set(&["--range", "time=2024-01-03T06:00:00Z.."], |row| {
        row.raw("time") >= "2024-01-03T06:00:00.000Z"
    });
    // Offsets are honoured: 01:00+01:00 is midnight UTC.
    assert_set(&["--range", "updated=..2024-01-02T01:00:00+01:00"], |row| {
        row.raw("updated") <= "2024-01-02T00:00:00.000Z"
    });
    let time = rows()[3].raw("time");
    assert_set(&["--term", &format!("time={time}")], |row| {
        row.raw("time") == time
    });
}

#[test]
fn geo_box() {
    let (min_lat, max_lat, min_lon, max_lon) = (30.0, 40.0, 130.0, 140.0);
    // No fixture point sits within quantisation reach of an edge.
    for row in rows() {
        let (lat, lon) = (row.f64("latitude").unwrap(), row.f64("longitude").unwrap());
        for edge in [lat - min_lat, lat - max_lat, lon - min_lon, lon - max_lon] {
            assert!(edge.abs() > 1e-5, "{} is on a box edge", row.id());
        }
    }
    assert_set(&["--box", "30,40,130,140"], |row| {
        in_range(row.f64("latitude"), min_lat, max_lat)
            && in_range(row.f64("longitude"), min_lon, max_lon)
    });
    // A box crossing the date line (min longitude > max longitude).
    assert_set(&["--box", "-60,60,170,-170"], |row| {
        let lon = row.f64("longitude").unwrap();
        in_range(row.f64("latitude"), -60.0, 60.0) && (lon >= 170.0 || lon <= -170.0)
    });
}

#[test]
fn geo_distance() {
    let (lat, lon, km) = (37.5, 137.2, 50.0);
    let distance = |row: &Row| {
        haversine_meters(
            lat,
            lon,
            row.f64("latitude").unwrap(),
            row.f64("longitude").unwrap(),
        )
    };
    for row in rows() {
        assert!(
            (distance(row) - km * 1000.0).abs() > 500.0,
            "{} is too close to the radius for an exact expectation",
            row.id()
        );
    }
    assert_set(&["--distance", "37.5,137.2,50"], |row| {
        distance(row) <= km * 1000.0
    });
}

// ---------------------------------------------------------------- combined

#[test]
fn combined_criteria() {
    // Text AND numeric range AND keyword.
    assert_set(
        &[
            "--text",
            "japan",
            "--range",
            "mag=4.5..",
            "--term",
            "magType=mb",
        ],
        |row| {
            row.place_words().contains(&"japan".to_owned())
                && in_range(row.f64("mag"), 4.5, f64::INFINITY)
                && row.raw("magType") == "mb"
        },
    );
    // Geo distance AND date range AND NOT a keyword.
    assert_set(
        &[
            "--distance",
            "37.5,137.2,100",
            "--range",
            "time=2024-01-01T12:00..2024-01-02",
            "--not",
            "--terms",
            "magType=mww,mwr",
        ],
        |row| {
            let d = haversine_meters(
                37.5,
                137.2,
                row.f64("latitude").unwrap(),
                row.f64("longitude").unwrap(),
            );
            d <= 100_000.0
                && ("2024-01-01T12:00:00.000Z"..="2024-01-02T00:00:00.000Z")
                    .contains(&row.raw("time"))
                && !["mww", "mwr"].contains(&row.raw("magType"))
        },
    );
    // Box AND int range AND double range.
    assert_set(
        &[
            "--box",
            "-90,90,-180,-60",
            "--range",
            "nst=10..",
            "--range",
            "depth=..20",
        ],
        |row| {
            in_range(row.f64("longitude"), -180.0, -60.0)
                && row.i64("nst").is_some_and(|nst| nst >= 10)
                && in_range(row.f64("depth"), f64::NEG_INFINITY, 20.0)
        },
    );
    // SHOULD-only clauses: any of them.
    assert_set(
        &[
            "--should", "--term", "net=ak", "--should", "--text", "vanuatu",
        ],
        |row| row.raw("net") == "ak" || row.place_words().contains(&"vanuatu".to_owned()),
    );
    // Only a prohibited clause: everything else.
    assert_set(&["--not", "--term", "net=us"], |row| row.raw("net") != "us");
}

// ---------------------------------------------------------------- sorting

fn sorted_rows(
    rows: Vec<&'static Row>,
    compare: impl Fn(&Row, &Row) -> Ordering,
) -> Vec<&'static Row> {
    let mut rows = rows;
    // Stable: ties keep fixture order, which is document order, as Lucene
    // breaks ties by document number.
    rows.sort_by(|a, b| compare(a, b));
    rows
}

#[test]
fn sort_by_one_field_both_directions() {
    let all = || matching(|_| true);
    for descending in [false, true] {
        let direction = if descending { "desc" } else { "asc" };
        assert_order(
            &["--sort", &format!("mag:{direction}")],
            &sorted_rows(all(), |a, b| {
                missing_last(a.f64("mag"), b.f64("mag"), descending)
            }),
        );
        assert_order(
            &["--sort", &format!("magError:{direction}")],
            &sorted_rows(all(), |a, b| {
                missing_last(a.f64("magError"), b.f64("magError"), descending)
            }),
        );
        assert_order(
            &["--sort", &format!("nst:{direction}")],
            &sorted_rows(all(), |a, b| {
                missing_last(a.i64("nst"), b.i64("nst"), descending)
            }),
        );
        assert_order(
            &["--sort", &format!("updated:{direction}")],
            &sorted_rows(all(), |a, b| {
                missing_last(Some(a.raw("updated")), Some(b.raw("updated")), descending)
            }),
        );
        assert_order(
            &["--sort", &format!("place:{direction}")],
            &sorted_rows(all(), |a, b| {
                missing_last(Some(a.raw("place")), Some(b.raw("place")), descending)
            }),
        );
    }
}

#[test]
fn sort_by_two_fields_both_directions() {
    let all = || matching(|_| true);
    assert_order(
        &["--sort", "net:asc,mag:desc"],
        &sorted_rows(all(), |a, b| {
            a.raw("net")
                .cmp(b.raw("net"))
                .then(missing_last(a.f64("mag"), b.f64("mag"), true))
        }),
    );
    assert_order(
        &["--sort", "magType:desc,time:asc"],
        &sorted_rows(all(), |a, b| {
            b.raw("magType")
                .cmp(a.raw("magType"))
                .then(a.raw("time").cmp(b.raw("time")))
        }),
    );
    // Filtered and sorted together, with a column that has missing values.
    assert_order(
        &["--text", "alaska", "--sort", "gap:asc,depth:desc"],
        &sorted_rows(
            matching(|row| row.place_words().contains(&"alaska".to_owned())),
            |a, b| {
                missing_last(a.f64("gap"), b.f64("gap"), false).then(missing_last(
                    a.f64("depth"),
                    b.f64("depth"),
                    true,
                ))
            },
        ),
    );
}

#[test]
fn sort_by_doc_and_distance() {
    assert_order(&["--sort", "_doc"], &matching(|_| true));
    let mut reversed = matching(|_| true);
    reversed.reverse();
    assert_order(&["--sort", "_doc:desc"], &reversed);

    let (lat, lon) = (37.5, 137.2);
    let distance = |row: &Row| {
        haversine_meters(
            lat,
            lon,
            row.f64("latitude").unwrap(),
            row.f64("longitude").unwrap(),
        )
    };
    let expected = sorted_rows(matching(|row| distance(row) <= 100_000.0), |a, b| {
        distance(a).total_cmp(&distance(b))
    });
    for pair in expected.windows(2) {
        assert!(
            distance(pair[1]) - distance(pair[0]) > 1.0,
            "distances too close for an exact order"
        );
    }
    assert_order(
        &["--distance", "37.5,137.2,100", "--sort", "_distance"],
        &expected,
    );
    // --origin gives the origin without filtering.
    let (hits, _) = search_ids(index(), &["--origin", "37.5,137.2", "--sort", "_distance"]);
    assert_eq!(&hits[..expected.len()], &ids(&expected)[..]);
}

#[test]
fn output_carries_selected_fields_sort_values_and_limit() {
    let output = run(&[
        "search",
        index().to_str().unwrap(),
        "--range",
        "mag=6..",
        "--sort",
        "mag:desc,time",
        "--fields",
        "id,mag,place",
        "--limit",
        "2",
    ]);
    let tsv = Tsv::parse(&output);
    let expected = sorted_rows(
        matching(|row| in_range(row.f64("mag"), 6.0, f64::INFINITY)),
        |a, b| {
            missing_last(a.f64("mag"), b.f64("mag"), true).then(a.raw("time").cmp(b.raw("time")))
        },
    );
    assert_eq!(tsv.total_hits, expected.len() as i64);
    assert_eq!(
        tsv.header,
        vec![
            "id",
            "mag",
            "place",
            "sort:mag:desc",
            "sort:time:asc",
            "score"
        ]
    );
    assert_eq!(tsv.rows.len(), 2);
    for (row, expected) in tsv.rows.iter().zip(&expected) {
        let mag = expected.f64("mag").unwrap().to_string();
        assert_eq!(row[0], expected.id());
        assert_eq!(row[1], mag);
        assert_eq!(row[2], expected.raw("place"));
        assert_eq!(row[3], mag);
        assert_eq!(row[4], expected.raw("time"));
    }
}

#[test]
fn relevance_is_the_default_order() {
    let (hits, total) = search_ids(index(), &["--text", "japan"]);
    let expected = matching(|row| row.place_words().contains(&"japan".to_owned()));
    assert_eq!(total, expected.len() as i64);
    assert_eq!(sorted(hits), sorted(ids(&expected)));
    let output = run(&[
        "search",
        index().to_str().unwrap(),
        "--text",
        "japan",
        "--limit",
        "1000",
    ]);
    let scores: Vec<f32> = Tsv::parse(&output)
        .column("score")
        .iter()
        .map(|score| score.parse().unwrap())
        .collect();
    assert!(
        scores.windows(2).all(|pair| pair[0] >= pair[1]),
        "scores descend"
    );
}

#[test]
fn invalid_requests_are_rejected() {
    let dir = index().to_str().unwrap();
    for (args, message) in [
        (vec!["--range", "magType=a..b"], "is not numeric or a date"),
        (vec!["--range", "mag=4"], "expects FIELD=LO..HI"),
        (vec!["--term", "bogus=1"], "unknown field \"bogus\""),
        (vec!["--sort", "_distance"], "_distance needs an origin"),
        (
            vec!["--origin", "1,2", "--sort", "_distance:desc"],
            "ascending only",
        ),
        (vec!["--sort", "mag:up"], "direction must be asc or desc"),
        (vec!["--not"], "must be followed by a criterion"),
        (vec!["--box", "1,2,3"], "expects 4 comma-separated numbers"),
        (vec!["--text", "  "], "produces no searchable terms"),
        (vec!["--limit", "0"], "--limit must be at least 1"),
    ] {
        let mut full = vec!["search", dir];
        full.extend(args.iter().copied());
        let error = run_err(&full);
        assert!(error.to_string().contains(message), "{args:?}: {error}");
    }
}
