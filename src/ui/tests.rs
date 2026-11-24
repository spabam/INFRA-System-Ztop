use super::*;
use crate::app::SortMode;
use crate::metrics::{DiskSnapshot, ProcRow, SNAPSHOT_SCHEMA_VERSION};
use ratatui::{backend::TestBackend, Terminal};
use std::collections::HashMap;

#[test]
fn renders_too_narrow_and_too_short_messages() {
    let snap = snapshot(0, 8, 2);
    assert!(render(39, 24, &mut AppState::default(), &snap).contains("terminal too narrow"));
    assert!(render(80, 9, &mut AppState::default(), &snap).contains("terminal too short"));
}

#[test]
fn crowded_servers_keep_processes_and_navigation_visible() {
    let snap = snapshot(8, 128, 40);
    let mut app = AppState::default();
    let rendered = render(80, 24, &mut app, &snap);
    assert!(rendered.contains("python worker-0"));
    assert!(rendered.contains("↑↓ scroll"));
    // Per-core rows are mandatory and share the screen with the processes.
    assert!(rendered.contains("15:"), "{rendered}");
    assert!(app.process_viewport_rows >= 5);
    assert!(app.gpu_page_count > 1);
    app.gpu_page = app.gpu_page_count - 1;
    let rendered = render(80, 24, &mut app, &snap);
    assert!(rendered.contains("GPU7"));
    assert!(rendered.contains("python worker-0"));
}

#[test]
fn sparse_gpu_panels_use_physical_index() {
    let mut snap = snapshot(1, 8, 2);
    snap.gpus[0].index = 7;
    let mut app = AppState::default();
    for height in [12, 24, 40] {
        let rendered = render(80, height, &mut app, &snap);
        assert!(rendered.contains("GPU7"));
        assert!(!rendered.contains("GPU0"));
    }
}

#[test]
fn table_header_marks_the_active_sort_column() {
    let snap = snapshot(1, 8, 2);
    let header = |sort: SortMode| {
        let mut app = AppState {
            sort,
            ..AppState::default()
        };
        render(80, 32, &mut app, &snap)
            .lines()
            .find(|line| line.contains("PID") && line.contains("COMMAND"))
            .unwrap()
            .to_string()
    };
    for (sort, marked) in [
        (SortMode::Cpu, "CPU%▼"),
        (SortMode::Ram, "RAM%▼"),
        (SortMode::Gpu, "GPU%▼"),
        (SortMode::Vram, "VRAM%▼"),
    ] {
        let line = header(sort);
        assert!(line.contains(marked), "{line}");
        assert_eq!(line.matches('▼').count(), 1, "{line}");
    }
    // Total is a derived score with no column of its own.
    assert!(!header(SortMode::Total).contains('▼'));
}

#[test]
fn compact_mode_reserves_process_rows_on_short_terminals() {
    let mut app = AppState::default();
    let rendered = render(40, 12, &mut app, &snapshot(8, 128, 10));
    assert!(rendered.contains("PID"));
    assert!(rendered.contains("worker-0"));
    assert!(app.process_viewport_rows >= 3);
}

#[test]
fn process_table_columns_follow_documented_breakpoints() {
    fn vram_cell(rendered: &str) -> &str {
        rendered
            .lines()
            .find(|line| line.contains("python worker-0"))
            .unwrap()
            .split_whitespace()
            .nth(5)
            .unwrap()
    }

    let snap = snapshot(1, 8, 2);
    for (width, user, gpu, vram) in [
        (80, true, true, true),
        (75, true, true, false),
        (69, true, false, false),
        (59, false, false, false),
    ] {
        let rendered = render(width, 32, &mut AppState::default(), &snap);
        let header = rendered
            .lines()
            .find(|line| line.contains("PID") && line.contains("CPU%"))
            .unwrap();
        assert_eq!(header.contains("USER"), user);
        assert_eq!(header.contains("GPU%"), gpu);
        assert_eq!(header.contains("VRAM%"), vram);
        if vram {
            // 512 MiB out of the fixture's 24 GiB, rounded like CPU/RAM.
            assert_eq!(vram_cell(&rendered), "2.1");
        }
    }

    let mut multi_gpu = snapshot(2, 8, 2);
    multi_gpu.gpus[0].vram_total_bytes = 8 * 1_073_741_824;
    multi_gpu.procs[0].vram_bytes = 8 * 1_073_741_824;
    // Sum both unequal GPU capacities: 8 GiB / (8 + 24) GiB = 25%.
    assert_eq!(
        vram_cell(&render(80, 32, &mut AppState::default(), &multi_gpu)),
        "25.0"
    );
    multi_gpu.procs[0].vram_bytes = 0;
    assert_eq!(
        vram_cell(&render(80, 32, &mut AppState::default(), &multi_gpu)),
        "—"
    );
    multi_gpu.procs[0].vram_bytes = 8 * 1_073_741_824;
    multi_gpu.gpus.clear();
    assert_eq!(
        vram_cell(&render(80, 32, &mut AppState::default(), &multi_gpu)),
        "—"
    );
}

#[test]
fn full_pid_and_large_cpu_usage_remain_visible() {
    let mut snap = snapshot(0, 128, 1);
    snap.procs[0].pid = 4_194_303;
    snap.procs[0].cpu_pct = 12_800.0;
    let rendered = render(80, 24, &mut AppState::default(), &snap);
    assert!(rendered.contains("4194303"));
    assert!(rendered.contains("12800.0"));
}

#[test]
fn filtering_precedes_process_limit_and_scrolling() {
    let snap = snapshot(0, 8, 40);
    let mut app = AppState {
        process_filter: "WORKER-3".into(),
        top_n: 3,
        ..AppState::default()
    };
    let rendered = render(80, 24, &mut app, &snap);
    assert!(rendered.contains("python worker-3 "));
    assert!(rendered.contains("python worker-30"));
    assert!(!rendered.contains("python worker-32"));
    assert_eq!(app.process_match_count, 11);
    app.process_filter.clear();
    app.top_n = 30;
    app.process_offset = 20;
    let rendered = render(80, 24, &mut app, &snap);
    assert!(rendered.contains("python worker-20"));
    assert!(!rendered.contains("python worker-0 "));
    app.process_filter = "no-such-process".into();
    let rendered = render(80, 24, &mut app, &snap);
    assert!(rendered.contains("No processes match"));
    assert_eq!(app.process_offset, 0);
}

#[test]
fn unicode_truncation_preserves_graphemes_and_terminal_width() {
    assert_eq!(truncate("abcdef", 0), "");
    assert_eq!(truncate("abcdef", 1), "…");
    assert_eq!(truncate("你好世界", 5), "你好…");
    assert_eq!(truncate("a👩‍💻bcd", 4), "a👩‍💻…");
    assert_eq!(truncate("e\u{301}cole", 4), "e\u{301}co…");
    for width in 0..10 {
        assert!(truncate("你好👩‍💻e\u{301}cole", width).width() <= width);
    }
}

#[test]
fn stale_and_unavailable_gpu_values_are_not_presented_as_fresh() {
    let mut snap = snapshot(2, 8, 2);
    snap.gpus[0].status = GpuSampleStatus::Stale;
    snap.gpus[0].sampled_at_unix_ms = snap.sampled_at_unix_ms - 1_250;
    snap.gpus[1].status = GpuSampleStatus::Unavailable;
    snap.gpus[1].util_pct = Some(99);
    snap.gpus[1].vram_used_bytes = 23 * 1_073_741_824;
    let rendered = render(100, 40, &mut AppState::default(), &snap);
    assert!(rendered.contains("stale 1.2s"));
    assert!(rendered.contains("unavailable"));
    assert!(!rendered.contains("99%"));
    assert!(!rendered.contains("23.0"));
    assert!(rendered.contains("VRAM n/a"));
}

#[test]
fn rows_left_over_by_the_panels_go_to_the_process_table() {
    let mut app = AppState {
        top_n: 40,
        ..AppState::default()
    };
    // Header (1), CPU (4 + one core row), RAM/swap (4), and two five-row GPU
    // panels use 20 of the 48 rows; the footer takes two and the table frame
    // and column header three. The other 23 are process rows: the fixture
    // has no disks, so nothing sits between the GPUs and the table.
    let rendered = render(100, 48, &mut app, &snapshot(2, 8, 40));
    assert_eq!(app.process_viewport_rows, 23);
    let rows = rendered.lines().collect::<Vec<_>>();
    assert!(rows[19].starts_with('└'), "{rendered}");
    assert!(rows[20].contains("Processes 1–23 / 40"), "{rendered}");
    assert!(rows[44].contains("python worker-22"), "{rendered}");
    // With a single GPU the table starts five rows higher still.
    render(100, 48, &mut app, &snapshot(1, 8, 40));
    assert_eq!(app.process_viewport_rows, 28);
    // Without a GPU the three-row placeholder is the only GPU panel.
    render(100, 48, &mut app, &snapshot(0, 8, 40));
    assert_eq!(app.process_viewport_rows, 30);
}

#[test]
fn disk_panel_sits_between_the_gpu_panels_and_the_process_table() {
    let mut snap = snapshot(1, 8, 40);
    snap.disks = vec![
        DiskSnapshot {
            model: Some("WD_BLACK SN850X 2000GB".into()),
            kind: Some("NVMe SSD".into()),
            temp_c: Some(52),
            ..disk("nvme0n1", 2000, 3, 441, 1740)
        },
        DiskSnapshot {
            kind: Some("SATA HDD".into()),
            ..disk("sda", 500, 1, 120, 465)
        },
    ];
    let mut app = AppState {
        top_n: 40,
        ..AppState::default()
    };
    let rendered = render(100, 48, &mut app, &snap);
    let rows = rendered.lines().collect::<Vec<_>>();
    // Header, CPU, RAM/swap and one GPU panel fill rows 0 to 14. Each disk
    // then takes two rows: what the drive is, and how full and warm it is.
    assert!(
        rows[15].starts_with("┌ Disks · 2 physical · 2500 GiB ─"),
        "{rendered}"
    );
    assert!(
        rows[16]
            .starts_with("│WD_BLACK SN850X 2000GB · NVMe SSD · nvme0n1 | 441 of 1740 GiB used "),
        "{rendered}"
    );
    assert!(
        rows[17].starts_with("│Disk   25.3%   temp: 52°C         ["),
        "{rendered}"
    );
    assert!(
        rows[18].starts_with("│SATA HDD · sda | 120.0 of 465.0 GiB used "),
        "{rendered}"
    );
    assert!(
        rows[19].starts_with("│Disk   25.8%   temp: n/a          ["),
        "{rendered}"
    );
    assert!(rows[20].starts_with('└'), "{rendered}");
    assert!(rows[21].contains("Processes 1–22 / 40"), "{rendered}");
    assert_eq!(app.process_viewport_rows, 22);
    // Every bar starts in the same column and ends at the frame: CPU, RAM,
    // GPU, VRAM and both disks.
    for row in [3, 7, 12, 13, 17, 19] {
        assert_eq!(
            rows[row].chars().position(|c| c == '['),
            Some(35),
            "{rendered}"
        );
        assert!(rows[row].ends_with("]│"), "{rendered}");
    }
}

#[test]
fn a_disk_with_nothing_mounted_says_so_and_has_no_percentage_or_bar() {
    let mut snap = snapshot(0, 8, 2);
    snap.disks = vec![
        disk("sda", 465, 1, 120, 450),
        DiskSnapshot {
            temp_c: Some(31),
            ..disk("sdb", 3726, 0, 0, 0)
        },
    ];
    let rendered = render(100, 40, &mut AppState::default(), &snap);
    let rows = rendered.lines().collect::<Vec<_>>();
    let described = rows
        .iter()
        .position(|line| line.starts_with("│sdb | no mounted filesystem "))
        .unwrap_or_else(|| panic!("{rendered}"));
    // The temperature stays in its column under the disk above.
    assert!(
        rows[described + 1].starts_with("│Disk       —   temp: 31°C "),
        "{rendered}"
    );
    assert!(!rows[described + 1].contains('['), "{rendered}");
    assert!(rendered.contains("│sda | 120.0 of 450.0 GiB used "));
    assert!(rendered.contains("│Disk   26.7%   temp: n/a          ["));
}

#[test]
fn a_disk_is_shown_in_the_shape_of_the_cpu_and_gpu_panels() {
    let mut snap = snapshot(1, 8, 2);
    snap.disks = vec![
        DiskSnapshot {
            model: Some("WD_BLACK SN850X 2000GB".into()),
            kind: Some("NVMe SSD".into()),
            temp_c: Some(52),
            ..disk("nvme0n1", 2000, 3, 441, 1740)
        },
        DiskSnapshot {
            model: Some("ST4000DM004-2CV1".into()),
            kind: Some("SATA HDD".into()),
            temp_c: Some(-3),
            ..disk("sda", 3726, 1, 3000, 3600)
        },
    ];
    let mut app = AppState::default();
    let buffer = render_buffer(100, 48, &mut app, &snap);
    let rows = buffer_rows(&buffer);
    let row = |start: &str| {
        rows.iter()
            .position(|line| line.starts_with(start))
            .unwrap_or_else(|| panic!("{start:?} not in\n{}", rows.join("\n")))
    };
    let (cpu, gpu) = (row("│CPU "), row("│GPU "));
    let disks = [row("│Disk   25.3%"), row("│Disk   83.3%")];

    // Above the figures stands what the device is, as the brand strings of
    // the CPU and GPU do, and the figures row names it by its kind alone.
    assert!(rows[cpu - 1].starts_with("│Test CPU "));
    assert!(rows[gpu - 1].starts_with("│RTX Test 0 "));
    assert!(rows[disks[0] - 1]
        .starts_with("│WD_BLACK SN850X 2000GB · NVMe SSD · nvme0n1 | 441 of 1740 GiB used "));
    assert!(rows[disks[1] - 1]
        .starts_with("│ST4000DM004-2CV1 · SATA HDD · sda | 3000 of 3600 GiB used "));
    assert!(rows[disks[0]].starts_with("│Disk   25.3%   temp: 52°C         ["));
    assert!(rows[disks[1]].starts_with("│Disk   83.3%   temp: -3°C         ["));
    assert!(!rows[disks[0]].contains("nvme0n1") && !rows[disks[1]].contains("sda"));

    // The percentage ends in the same column on the CPU and disk rows, and
    // every bar starts and ends where the CPU and GPU bars do.
    let column = |line: usize, symbol: char| rows[line].chars().position(|c| c == symbol);
    assert_eq!(column(disks[0], '%'), column(cpu, '%'));
    for line in [gpu, disks[0], disks[1]] {
        assert_eq!(column(line, '['), column(cpu, '['), "{}", rows[line]);
        assert!(rows[line].ends_with("]│"), "{}", rows[line]);
    }

    // The same colours on all three: the device line like the brand strings,
    // the label like every label, the reading in the temperature colour.
    let colours = |line: usize, text: &str| {
        let cells = rows[line].chars().collect::<Vec<_>>();
        let needle = text.chars().collect::<Vec<_>>();
        let at = cells
            .windows(needle.len())
            .position(|window| window == needle.as_slice())
            .unwrap_or_else(|| panic!("{text:?} not in {:?}", rows[line]));
        (at..at + needle.len())
            .map(|x| buffer[(x as u16, line as u16)].fg)
            .collect::<Vec<_>>()
    };
    let all = |line: usize, text: &str, colour: Color| {
        assert!(
            colours(line, text).iter().all(|&fg| fg == colour),
            "{text:?} in {:?}",
            rows[line]
        );
    };
    all(cpu - 1, "Test CPU", Color::Gray);
    all(gpu - 1, "RTX Test 0", Color::Gray);
    all(
        disks[0] - 1,
        "WD_BLACK SN850X 2000GB · NVMe SSD · nvme0n1",
        Color::Gray,
    );
    for (line, label, reading) in [
        (cpu, "CPU", "67.0°C"),
        (gpu, "GPU", "71°C"),
        (disks[0], "Disk", "52°C"),
        (disks[1], "Disk", "-3°C"),
    ] {
        all(line, label, Color::Cyan);
        all(line, "temp", Color::Cyan);
        all(line, reading, Color::Yellow);
    }
}

#[test]
fn a_disk_without_model_kind_or_sensor_still_has_both_rows() {
    let mut snap = snapshot(1, 8, 2);
    snap.disks = vec![disk("vda", 100, 1, 25, 100)];
    let mut app = AppState::default();
    let rendered = render(100, 48, &mut app, &snap);
    // Only the kernel name is known: it heads the disk, and the reading is
    // n/a as it is for a CPU without a sensor.
    assert!(
        rendered.contains("│vda | 25.0 of 100.0 GiB used "),
        "{rendered}"
    );
    assert!(
        rendered.contains("│Disk   25.0%   temp: n/a          ["),
        "{rendered}"
    );
    // A long description is cut at the frame; the figures keep their place.
    snap.disks[0].model = Some("A drive with a remarkably long model name".into());
    snap.disks[0].kind = Some("virtual disk".into());
    let narrow = render(50, 48, &mut app, &snap);
    assert!(
        narrow.contains("│A drive with a remarkably long model name · vir…│"),
        "{narrow}"
    );
    assert!(
        narrow.contains("│Disk   25.0%   temp: n/a          [███░░░░░░░░░]│"),
        "{narrow}"
    );
}

#[test]
fn the_fullest_disks_are_listed_when_rows_run_out() {
    let mut snap = snapshot(1, 8, 2);
    snap.disks = vec![
        disk("sda", 1000, 1, 100, 1000),
        disk("sdb", 1000, 1, 900, 1000),
        disk("sdc", 1000, 0, 0, 0),
        disk("sdd", 1000, 1, 500, 1000),
        disk("sde", 1000, 2, 950, 1000),
        disk("sdf", 1000, 1, 700, 1000),
    ];
    let mut app = AppState::default();
    let listed = |rendered: &str| {
        rendered
            .lines()
            .filter_map(|line| line.strip_prefix("│sd"))
            .map(|line| line.chars().next().unwrap())
            .collect::<String>()
    };
    // A tall terminal still lists four at most, in name order.
    let rendered = render(100, 48, &mut app, &snap);
    assert!(
        rendered.contains(" Disks · 6 physical · 6000 GiB · 4 fullest shown "),
        "{rendered}"
    );
    assert_eq!(listed(&rendered), "bdef", "{rendered}");
    // Thirty-one rows leave the two rows of one disk beside ten for the
    // process table: the fullest disk is the one listed.
    let rendered = render(100, 31, &mut app, &snap);
    assert!(rendered.contains(" · 1 fullest shown "), "{rendered}");
    assert_eq!(listed(&rendered), "e", "{rendered}");
    assert!(
        rendered.contains("│sde | 950 of 1000 GiB used "),
        "{rendered}"
    );
    assert!(
        rendered.contains("│Disk   95.0%   temp: n/a "),
        "{rendered}"
    );
    assert_eq!(app.process_viewport_rows, 7);
    // A second disk needs two more rows.
    assert_eq!(listed(&render(100, 32, &mut app, &snap)), "e");
    assert_eq!(listed(&render(100, 33, &mut app, &snap)), "be");
    // One row fewer than thirty-one and the process table keeps them all.
    let rendered = render(100, 30, &mut app, &snap);
    assert!(!rendered.contains("Disks"), "{rendered}");
    assert_eq!(app.process_viewport_rows, 10);
    // Compact and 80x24 layouts never had room for the panel.
    assert!(!render(80, 24, &mut app, &snap).contains("Disks"));
    assert!(!render(80, 12, &mut app, &snap).contains("Disks"));
}

#[test]
fn capacities_stay_five_columns_wide_across_units() {
    const GIB: u64 = 1 << 30;
    let tenth = GIB / 10;
    assert_eq!(capacity_pair(164 * tenth, 939 * tenth), " 16.4 /  93.9 GiB");
    assert_eq!(capacity_pair(441 * GIB, 1740 * GIB), "  441 /  1740 GiB");
    // The decimal goes exactly where it would need a sixth column.
    assert_eq!(capacity_pair(0, 9999 * tenth), "  0.0 / 999.9 GiB");
    assert_eq!(capacity_pair(0, 99_996 * GIB / 100), "    0 /  1000 GiB");
    assert_eq!(
        capacity_pair(99_999 * GIB, 99_999 * GIB),
        "99999 / 99999 GiB"
    );
    assert_eq!(
        capacity_pair(100_000 * GIB, 200_000 * GIB),
        " 97.7 / 195.3 TiB"
    );
    assert_eq!(capacity(1863 * GIB), "1863 GiB");
    assert_eq!(capacity(4658 * tenth), "465.8 GiB");
    // A disk's sizes are written in the unit and precision of its capacity.
    let sized = |used: u64, total: u64| {
        disk_description(&DiskSnapshot {
            fs_used_bytes: used,
            fs_total_bytes: total,
            ..disk("sda", 1, 1, 0, 1)
        })
    };
    assert_eq!(
        sized(4411 * tenth, 9168 * tenth),
        "sda | 441.1 of 916.8 GiB used"
    );
    assert_eq!(sized(441 * GIB, 1740 * GIB), "sda | 441 of 1740 GiB used");
    assert_eq!(
        sized(100_000 * GIB, 200_000 * GIB),
        "sda | 97.7 of 195.3 TiB used"
    );
}

#[test]
fn disk_panel_title_falls_back_to_the_bare_label_when_it_does_not_fit() {
    let disks = [disk("sda", 500, 1, 1, 2), disk("sdb", 500, 1, 1, 2)];
    assert_eq!(
        disks_title(&disks, 2, 40),
        " Disks · 2 physical · 1000 GiB "
    );
    assert_eq!(
        disks_title(&disks, 1, 60),
        " Disks · 2 physical · 1000 GiB · 1 fullest shown "
    );
    assert_eq!(disks_title(&disks, 1, 40), " Disks ");
}

#[test]
fn core_layout_uses_available_width_and_cannot_be_hidden() {
    assert_eq!(core_cell_width(8), 7);
    assert_eq!(core_cell_width(128), 8);
    assert_eq!(cores_per_row(80, 128), 8);
    assert_eq!(cores_per_row(160, 128), 17);
    assert_eq!(cores_per_row(10, 128), 1);
    let mut app = AppState::default();
    assert!(render(160, 40, &mut app, &snapshot(0, 128, 2)).contains("127:"));
    // A 24-row terminal still shows every core of a 14-thread host next to
    // the process table: the rows outrank the GPU panel height.
    let rendered = render(80, 24, &mut app, &snapshot(1, 14, 2));
    assert!(rendered.contains("13:"), "{rendered}");
    assert!(rendered.contains("python worker-0"), "{rendered}");
}

#[test]
fn help_explains_search_scrolling_and_gpu_pages() {
    let mut app = AppState {
        show_help: true,
        ..AppState::default()
    };
    let rendered = render(80, 24, &mut app, &snapshot(1, 8, 2));
    for text in ["Keys", "Ctrl-C", "search command", "PgUp PgDn", "GPU page"] {
        assert!(rendered.contains(text));
    }
}

#[test]
fn bars_fit_when_metric_prefix_grows() {
    let mut spans = Vec::new();
    append_aligned_bar(&mut spans, 40, 100.0, Rect::new(0, 0, 50, 1));
    assert_eq!(spans.iter().map(|span| span.width()).sum::<usize>(), 8);
}

#[test]
fn cpu_panel_title_shows_core_and_thread_counts() {
    let mut app = AppState::default();
    let mut snap = snapshot(0, 8, 2);
    let rendered = render(80, 40, &mut app, &snap);
    assert!(
        rendered.contains(" CPU · 4 cores / 8 threads "),
        "{rendered}"
    );
    assert!(rendered.contains("Test CPU"));
    // Short terminals use the compact summary, which carries the same counts.
    assert!(render(80, 12, &mut app, &snap).contains("4 cores / 8 threads"));
    // Platforms that hide the physical topology still report the thread count.
    snap.cpu_physical_cores = None;
    assert!(render(80, 40, &mut app, &snap).contains(" CPU · 8 threads "));
    // Auto-fit limits the per-core rows silently; the counts stay in the title.
    let rendered = render(80, 24, &mut app, &snapshot(0, 128, 2));
    assert!(
        rendered.contains(" CPU · 64 cores / 128 threads "),
        "{rendered}"
    );
    assert!(!rendered.contains("auto-fit"), "{rendered}");
}

#[test]
fn cpu_panel_title_falls_back_to_the_bare_label_when_counts_do_not_fit() {
    let snap = snapshot(0, 8, 2);
    assert_eq!(cpu_title(&snap, 40), " CPU · 4 cores / 8 threads ");
    assert_eq!(cpu_title(&snap, 20), " CPU ");
}

fn snapshot(gpu_count: usize, cores: usize, processes: usize) -> Snapshot {
    Snapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        sampled_at_unix_ms: 1_783_261_200_000,
        hostname: "host".into(),
        uptime_secs: 3_723,
        cpu_brand: "Test CPU".into(),
        cpu_physical_cores: Some(cores.div_ceil(2).max(1)),
        cpu_total_pct: 42.0,
        cpu_per_core_pct: (0..cores).map(|i| (i % 100) as f32).collect(),
        cpu_temp_c: Some(67.0),
        ram_used_bytes: 8 * 1_073_741_824,
        ram_total_bytes: 16 * 1_073_741_824,
        swap_used_bytes: 0,
        swap_total_bytes: 0,
        gpus: (0..gpu_count)
            .map(|index| GpuSnapshot {
                index: index as u32,
                id: format!("test-gpu-{index}"),
                name: format!("RTX Test {index}"),
                sampled_at_unix_ms: 1_783_261_200_000,
                status: GpuSampleStatus::Fresh,
                util_pct: Some(42),
                util_peak_pct: Some(42),
                temp_c: Some(71),
                vram_used_bytes: 4 * 1_073_741_824,
                vram_peak_used_bytes: Some(4 * 1_073_741_824),
                vram_total_bytes: 24 * 1_073_741_824,
                per_pid: HashMap::new(),
            })
            .collect(),
        disks: Vec::new(),
        procs: (0..processes)
            .map(|index| ProcRow {
                pid: 100 + index as u32,
                user: "alice".into(),
                cpu_pct: 10.0,
                ram_pct: 2.0,
                gpu_pct: Some(5),
                vram_bytes: 512 * 1_048_576,
                command: format!("python worker-{index}"),
            })
            .collect(),
    }
}

/// A disk of `size_gib` whose mounted filesystems hold `used_gib` of
/// `total_gib`, of which the kernel knows neither model, kind nor
/// temperature.
fn disk(
    name: &str,
    size_gib: u64,
    filesystems: u32,
    used_gib: u64,
    total_gib: u64,
) -> DiskSnapshot {
    const GIB: u64 = 1 << 30;
    DiskSnapshot {
        name: name.into(),
        model: None,
        kind: None,
        size_bytes: size_gib * GIB,
        filesystems,
        fs_used_bytes: used_gib * GIB,
        fs_total_bytes: total_gib * GIB,
        fs_available_bytes: (total_gib - used_gib) * GIB,
        temp_c: None,
    }
}

fn render(width: u16, height: u16, app: &mut AppState, snap: &Snapshot) -> String {
    buffer_rows(&render_buffer(width, height, app, snap)).join("\n")
}

/// The drawn frame with its styles, for checks that the text cannot make.
fn render_buffer(
    width: u16,
    height: u16,
    app: &mut AppState,
    snap: &Snapshot,
) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| {
            prepare_frame(app, snap, f.area());
            draw(f, app, snap);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn buffer_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}
