use crate::Fixture;

#[test]
fn defaults_come_from_the_config_file() {
    let fixture = Fixture::new();
    fixture.outside("syk/config", "# defaults\n-E route.ts\n--stats\n");
    assert_eq!(
        fixture.lines(&["-c", "needle"]),
        ["src/api/auth.ts:2:const needle = 1"]
    );
    assert_eq!(fixture.lines(&["-c", "needle", "-U"]).len(), 2);
    assert!(
        fixture
            .errors(&["-c", "needle"])
            .contains("1 line in 1 file")
    );
    assert!(
        fixture
            .run(&["-c", "needle", "-x"])
            .0
            .contains("defaults  -E route.ts --stats")
    );
    assert_eq!(fixture.run(&["-h"]).1, 0);
    assert_eq!(fixture.run(&[]).1, 2);

    fixture.outside("syk/config", "needle\n");
    assert_eq!(fixture.run(&["-c", "needle"]).1, 2);
    assert!(fixture.errors(&["-c", "needle"]).contains("is not a flag"));
    assert_eq!(fixture.run(&["-c", "needle", "-U"]).1, 0);
}
