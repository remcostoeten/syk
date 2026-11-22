use crate::Fixture;

fn ranking_fixture() -> Fixture {
    let fixture = Fixture::new();
    for name in ["config", "config.ts", "app_config.ts", "myconfig.ts"] {
        fixture.file(&format!("pick/{name}"), "needle\n");
    }
    fixture
}

#[test]
fn ranking_orders_names_by_how_well_they_match() {
    let fixture = ranking_fixture();
    let ranked = |args: &[&str]| -> Vec<String> {
        fixture.run(args).0.lines().map(str::to_string).collect()
    };
    assert_eq!(
        ranked(&["-n", "config", "-r"]),
        [
            "pick/config",
            "pick/config.ts",
            "pick/app_config.ts",
            "pick/myconfig.ts",
        ]
    );
    assert_eq!(
        ranked(&["-n", "config", "-r", "-m", "2"]),
        ["pick/config", "pick/config.ts"]
    );
    assert_eq!(ranked(&["-n", "config", "-1"]), ["pick/config"]);
    assert_eq!(ranked(&["-c", "needle", "-1", "pick/"]).len(), 1);
    assert_eq!(fixture.run(&["-n", "zzzmissing", "-1"]), (String::new(), 1));
}

#[test]
fn a_picked_path_ranks_higher_afterwards() {
    let fixture = ranking_fixture();
    let best = |pattern: &str| fixture.run(&["-n", pattern, "-1", "-t", "file"]).0;
    assert_eq!(
        fixture.run(&["-n", "config", "-r", "-m", "1"]).0,
        "pick/config\n"
    );
    assert_eq!(best("myconfig"), "pick/myconfig.ts\n");
    assert_eq!(best("config"), "pick/myconfig.ts\n");
    assert_eq!(
        fixture.run(&["-n", "config", "-r", "-m", "1"]).0,
        "pick/myconfig.ts\n"
    );
}

#[test]
fn a_picked_hidden_path_is_found_without_asking_for_hidden_files() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run(&["-n", "secret", "-r"]), (String::new(), 1));
    assert_eq!(
        fixture.run(&["-n", "secret", "-H", "-1"]).0,
        ".hidden/secret.txt\n"
    );
    assert_eq!(
        fixture.run(&["-n", "secret", "-r"]).0,
        ".hidden/secret.txt\n"
    );
    assert_eq!(
        fixture.run(&["-n", "secret", "-1"]).0,
        ".hidden/secret.txt\n"
    );
    assert_eq!(fixture.run(&["-n", "secret", "-R"]), (String::new(), 1));
    assert_eq!(fixture.run(&["-d", "secret", "-r"]), (String::new(), 1));
    assert_eq!(
        fixture.run(&["-n", "secret", "-r", "src/"]),
        (String::new(), 1)
    );
}

#[test]
fn scores_and_picks_can_be_inspected() {
    let fixture = ranking_fixture();
    let (empty, status) = fixture.run(&["-k"]);
    assert_eq!(status, 0);
    assert!(empty.starts_with("Nothing is remembered yet."));

    let (why, _) = fixture.run(&["-n", "config", "-W", "-m", "2"]);
    let rows: Vec<&str> = why.lines().collect();
    assert!(rows[0].starts_with("score = picked (max 4) + name (max 3)"));
    assert!(
        rows[1]
            .trim_start()
            .starts_with("score  picked   name  visited  recent  shallow  path")
    );
    assert!(rows[2].ends_with("  3.00     0.00    1.00     0.17  pick/config"));
    assert!(rows[3].ends_with("  2.70     0.00    1.00     0.17  pick/config.ts"));
    assert_eq!(rows.len(), 4);

    let (dry, _) = fixture.run(&["-n", "myconfig", "-1", "-W"]);
    assert!(dry.ends_with("pick/myconfig.ts\n"));
    assert!(
        fixture
            .run(&["-k"])
            .0
            .starts_with("Nothing is remembered yet.")
    );

    fixture.run(&["-n", "myconfig", "-1"]);
    let (picks, _) = fixture.run(&["-k"]);
    let listed: Vec<&str> = picks.lines().collect();
    assert!(
        listed[0]
            .trim_start()
            .starts_with("score  picks  last picked  path")
    );
    assert!(listed[1].starts_with("  4.00    1.0  just now   "));
    assert!(listed[1].ends_with("/pick/myconfig.ts"));
    let (after, _) = fixture.run(&["-n", "config", "-W", "-m", "1"]);
    assert!(
        after
            .lines()
            .nth(2)
            .is_some_and(|row| row.contains("  4.00   0.60 "))
    );
}
