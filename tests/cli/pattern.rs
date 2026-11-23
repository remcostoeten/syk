use crate::Fixture;

#[test]
fn patterns_are_forgiving() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["-c", "login("]),
        ["src/api/auth.ts:1:export function login() {}"]
    );
    assert_eq!(fixture.lines(&["*.tsx", "-n"]), ["src/web/route.tsx"]);
    assert_eq!(fixture.lines(&["-n", "*oute.ts"]), ["src/api/route.ts"]);
    assert_eq!(fixture.lines(&["-c", "NEEDLE"]), Vec::<String>::new());
    assert_eq!(fixture.lines(&["-c", "NEEDLE", "-i", "-l"]).len(), 2);
    assert_eq!(fixture.lines(&["-c", "need", "-w"]), Vec::<String>::new());
}

#[test]
fn several_words_have_to_match_together() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["route", "in"]),
        ["src/api/route.ts:1:needle in route"]
    );
    assert_eq!(
        fixture.lines(&["const", "= 1"]),
        ["src/api/auth.ts:2:const needle = 1"]
    );
    assert_eq!(
        fixture.lines(&["tsx", "route", "-n"]),
        ["src/web/route.tsx"]
    );
    assert_eq!(fixture.run(&["other", "zzmissing"]).1, 1);
}

#[test]
fn an_empty_word_is_refused() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run(&[""]).1, 2);
    assert!(fixture.errors(&["", "src/"]).contains("is empty"));
    assert_eq!(fixture.run(&["--", ""]).1, 2);
    assert!(!fixture.lines(&["-n"]).is_empty());
}
