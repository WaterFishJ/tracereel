//! Opt-in real model regressions: set TRACEREEL_OCR_SAMPLE for the original
//! sample, or TRACEREEL_OCR_SEPARATOR_SAMPLE for the pipe-separated park receipt.

#[test]
#[ignore = "requires local OCR assets and the pipe-separated park receipt"]
fn pipe_separator_does_not_become_a_seconds_digit() {
    let request = receipt_ocr::Request {
        path: std::env::var_os("TRACEREEL_OCR_SEPARATOR_SAMPLE")
            .expect("set TRACEREEL_OCR_SEPARATOR_SAMPLE")
            .into(),
        corners: [
            [0.024, 0.092],
            [0.175, 0.094],
            [0.17, 0.493],
            [0.021, 0.489],
        ],
        rotation: 0,
    };
    let mut engine = receipt_ocr::Engine::new(&receipt_ocr::runtime_dir().unwrap()).unwrap();
    let cumulative = [
        "00:00:00", "00:03:02", "00:09:22", "00:21:23", "00:23:37", "00:24:34", "00:27:03",
        "00:29:46", "00:32:49", "00:35:08", "00:37:40", "00:44:20", "00:46:52", "00:48:08",
        "00:51:56", "00:53:46", "00:54:37", "00:57:17", "00:59:47", "01:03:02", "01:04:23",
        "01:05:36",
    ];
    let splits = [
        "00:00:00", "00:03:02", "00:06:20", "00:12:01", "00:02:14", "00:00:57", "00:02:29",
        "00:02:43", "00:03:03", "00:02:19", "00:02:32", "00:06:40", "00:02:32", "00:01:16",
        "00:03:48", "00:01:50", "00:00:51", "00:02:40", "00:02:30", "00:03:15", "00:01:21",
        "00:01:13",
    ];
    for _ in 0..2 {
        let result = engine.recognize(&request).unwrap();
        assert_eq!(result.columns, 4);
        assert_eq!(result.rows.len(), 22);
        for (i, row) in result.rows.iter().enumerate() {
            assert_eq!(row.cells[0], format!("{:03}", i + 1));
            assert_eq!(row.cells[2], cumulative[i], "cumulative row {}", i + 1);
            assert_eq!(row.cells[3], splits[i], "split row {}", i + 1);
        }
        // Keep the true digit 1 in control codes and valid second values.
        assert_eq!(result.rows[1].cells[1], "0031");
        assert_eq!(result.rows[11].cells[1], "0041");
        assert_eq!(result.rows[3].cells[3], "00:12:01");
        assert!(result.rows[1].scores[2] < 0.85);
    }
}
#[test]
#[ignore = "requires local OCR assets and the user's sample image"]
fn sample_receipt_times_and_cached_model_reuse() {
    let path = std::env::var_os("TRACEREEL_OCR_SAMPLE").expect("set TRACEREEL_OCR_SAMPLE");
    let request = receipt_ocr::Request {
        path: path.into(),
        corners: [
            [0.335, 0.485],
            [0.676, 0.483],
            [0.639, 0.863],
            [0.335, 0.85],
        ],
        rotation: 0,
    };
    let mut engine = receipt_ocr::Engine::new(&receipt_ocr::runtime_dir().unwrap()).unwrap();
    let cumulative = [
        "00:01:21",
        "00:01:45",
        "00:03:36",
        "00:04:07",
        "00:04:49",
        "00:05:46",
        "00:08:00",
        "00:08:43",
        "00:09:31",
        "00:12:45",
        "00:15:03",
        "00:17:21",
        "00:18:01",
        "00:19:45",
        "00:20:04",
        "00:20:21",
        "00:20:53",
        "00:21:35",
        "00:22:03.1",
    ];
    let splits = [
        "00:01:21", "00:00:24", "00:01:51", "00:00:31", "00:00:42", "00:00:57", "00:02:14",
        "00:00:43", "00:00:48", "00:03:14", "00:02:18", "00:02:18", "00:00:40", "00:01:44",
        "00:00:19", "00:00:17", "00:00:32", "00:00:42", "00:00:28",
    ];
    for _ in 0..2 {
        let result = engine.recognize(&request).unwrap();
        assert_eq!(result.columns, 4);
        assert_eq!(result.rows.len(), 21);
        for (i, row) in result.rows[2..].iter().enumerate() {
            assert_eq!(row.cells[2], cumulative[i]);
            assert_eq!(row.cells[3], splits[i]);
        }
        assert_eq!(result.rows[13].cells[1], "120");
        assert_eq!(result.rows[19].cells[1], "120");
    }
}
