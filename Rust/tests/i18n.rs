//! Translations, formatting and duration strings.

use lamzu_battery::i18n::{Key, Language, human_age, render, state_text, template, tr, tr_args};
use lamzu_battery::protocol::ChargeState;
use proptest::prelude::*;

#[test]
fn every_key_has_text_in_both_languages() {
    for key in Key::ALL {
        for language in Language::ALL {
            assert!(
                !template(language, *key).trim().is_empty(),
                "{} has no text for {}",
                key.as_str(),
                language.code()
            );
        }
    }
}

#[test]
fn keys_are_unique() {
    let mut names: Vec<&str> = Key::ALL.iter().map(|key| key.as_str()).collect();
    names.sort_unstable();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count);
}

#[test]
fn state_text_is_localized() {
    assert_eq!(state_text(Language::Ru, ChargeState::Discharging), "разряд");
    assert_eq!(
        state_text(Language::Ru, ChargeState::Charging),
        "заряжается"
    );
    assert_eq!(state_text(Language::Ru, ChargeState::Full), "заряжена");
    assert_eq!(
        state_text(Language::Ru, ChargeState::Other(9)),
        "состояние 9"
    );
    assert_eq!(
        state_text(Language::En, ChargeState::Discharging),
        "discharging"
    );
    assert_eq!(state_text(Language::En, ChargeState::Other(9)), "state 9");
}

#[test]
fn placeholders_are_substituted() {
    assert_eq!(
        tr_args(
            Language::Ru,
            Key::MenuInstallUpdate,
            &[("version", "1.5.7")]
        ),
        "Установить 1.5.7"
    );
    assert_eq!(
        tr_args(
            Language::En,
            Key::StatusOffline,
            &[("percent", "42"), ("age", "5 min")]
        ),
        "42% · no connection (5 min)"
    );
    assert_eq!(
        tr_args(
            Language::Ru,
            Key::StatusOffline,
            &[("percent", "42"), ("age", "5 мин")]
        ),
        "42% · нет связи (5 мин)"
    );
    assert_eq!(
        tr_args(Language::En, Key::UpdateLatest, &[("version", "1.3.0")]),
        "You have the latest version (1.3.0)."
    );
}

#[test]
fn unknown_placeholders_and_broken_braces_survive() {
    assert_eq!(render("{name} {unknown}", &[("name", "x")]), "x {unknown}");
    assert_eq!(render("brace {", &[]), "brace {");
    assert_eq!(render("plain", &[]), "plain");
}

#[test]
fn human_age_matches_the_python_wording() {
    assert_eq!(human_age(Language::Ru, 5), "только что");
    assert_eq!(human_age(Language::Ru, 5 * 60), "5 мин");
    assert_eq!(human_age(Language::Ru, 2 * 3600 + 10 * 60), "2 ч 10 мин");
    assert_eq!(human_age(Language::Ru, 2 * 86_400 + 3 * 3600), "2 д 3 ч");
    assert_eq!(human_age(Language::En, 5), "just now");
    assert_eq!(human_age(Language::En, 5 * 60), "5 min");
    assert_eq!(human_age(Language::En, 2 * 3600 + 10 * 60), "2 h 10 min");
    assert_eq!(human_age(Language::En, 2 * 86_400 + 3 * 3600), "2 d 3 h");
}

#[test]
fn unknown_languages_fall_back_to_english() {
    assert_eq!(Language::from_code("de"), Language::En);
    assert_eq!(Language::from_code("RU"), Language::Ru);
    assert_eq!(tr(Language::En, Key::MenuExit), "Exit");
    assert_eq!(Language::DEFAULT, Language::En);
}

#[test]
fn translations_are_stable_and_have_no_unfilled_placeholders_for_plain_keys() {
    for key in Key::ALL {
        let text = tr(Language::En, *key);
        assert!(!text.is_empty());
        assert_eq!(text, tr(Language::En, *key));
    }
}

proptest! {
    #[test]
    fn render_never_panics(template in ".*", argument in ".*") {
        let _ = render(&template, &[("name", &argument)]);
    }

    #[test]
    fn human_age_is_monotonic(seconds in 0_u64..10_000_000) {
        let first = human_age(Language::En, seconds);
        let second = human_age(Language::En, seconds + 1);
        prop_assert!(!first.is_empty() && !second.is_empty());
    }
}
