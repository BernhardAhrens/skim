//! Which language is this message written in?
//!
//! Answered locally, by trigrams, in tens of microseconds — the reading pane
//! asks on every body render to decide whether to offer a translation, so this
//! must never cost a network round-trip and never be worth caching.
//!
//! Silence is the safe answer: a body that is too short, too noisy, or simply
//! ambiguous offers nothing.

use crate::mail::parse::{strip_css, strip_quoted};
use std::sync::OnceLock;
use whatlang::{Detector, Lang, Script};

/// Taken off the front of the body before any other work, so a 500 KB
/// newsletter isn't split into lines and rejoined on every open.
const HEAD_CHARS: usize = 8_000;
/// What the detector actually sees. Trigram scoring saturates long before this.
const SAMPLE_CHARS: usize = 2_000;
/// Below this many letters the guess isn't worth making. Kept well under a
/// Latin paragraph because CJK says the same thing in a third of the
/// characters, and a threshold tuned for English would skip short Japanese
/// mail entirely — the case the feature exists for.
const MIN_LETTERS: usize = 60;

/// The languages worth considering: every locale Skim's UI speaks, plus the
/// ones a European mailbox actually receives. Unrestricted, whatlang cheerfully
/// files short English text under Tagalog or Esperanto; an allowlist removes
/// that failure mode. A language genuinely outside the list gets misfiled as a
/// neighbour, which is harmless — the *target* language always comes from the
/// user's locale, never from this guess.
const ALLOWED: &[Lang] = &[
    Lang::Eng,
    Lang::Rus,
    Lang::Srp,
    // whatlang files Serbian under Cyrillic only; the Latin spelling of the
    // very same language — and of Bosnian and Croatian — scores as `Hrv`.
    // Without it here, Latin-script Serbian has no right answer to win with,
    // its neighbours split the vote, and the pane stays silent.
    Lang::Hrv,
    Lang::Fra,
    Lang::Deu,
    Lang::Spa,
    Lang::Ita,
    Lang::Pol,
    Lang::Cmn,
    Lang::Jpn,
    Lang::Kor,
    Lang::Nld,
    Lang::Por,
    Lang::Tur,
    Lang::Ukr,
    Lang::Ces,
    Lang::Swe,
    Lang::Ara,
    Lang::Heb,
    Lang::Hin,
];

fn detector() -> &'static Detector {
    static DETECTOR: OnceLock<Detector> = OnceLock::new();
    DETECTOR.get_or_init(|| Detector::with_allowlist(ALLOWED.to_vec()))
}

/// What can be said about a body: the language, when the guess is solid, or
/// else the writing system alone — which is still worth something.
enum Detected {
    Lang(&'static str),
    Script(Script),
}

/// Is this body in a language the reader can't read? The one question the
/// reading pane actually has: `locale` is the language of their UI.
pub fn is_foreign(body_text: &str, locale: &str) -> bool {
    // Detection speaks bare ISO 639-1; the setting normally does too, but
    // compare primary subtags so an "en-US" still matches English.
    let own = locale.split(['-', '_']).next().unwrap_or(locale);
    match detect(body_text) {
        Some(Detected::Lang(code)) => code != own,
        // Too close to call: the language is outside the list above, or two
        // neighbours split the vote. The script is still solid, and someone
        // whose own language isn't written in it can't read this mail whichever
        // of the candidates it turns out to be.
        Some(Detected::Script(script)) => !writes(own, script),
        None => false,
    }
}

/// The body's language, or the script it is written in when the language is
/// too close to call. `None` when even the script says nothing.
fn detect(body_text: &str) -> Option<Detected> {
    let head: String = body_text.chars().take(HEAD_CHARS).collect();
    // Some senders' text part is their HTML with the tags taken out, `<style>`
    // included, so the body opens with a stylesheet. Braces and property names
    // are not a language, and there are enough of them to outvote the prose.
    let head = strip_css(&head);
    // Quoted tails are usually the *other* party's language, so prefer the
    // sender's own words — but a forward or a bottom-posted reply has none, and
    // that's exactly the mail most worth translating. Fall back to the raw head.
    let stripped = strip_quoted(&head);
    let source = if letters(&stripped) >= MIN_LETTERS {
        stripped
    } else {
        head
    };

    let sample = sample(&source);
    if letters(&sample) < MIN_LETTERS {
        return None;
    }
    let info = detector().detect(&sample)?;
    Some(match iso1(info.lang()).filter(|_| info.is_reliable()) {
        Some(code) => Detected::Lang(code),
        None => Detected::Script(info.script()),
    })
}

/// Is `lang` (ISO 639-1) written in `script`? Only asked when the language
/// guess didn't hold, to find out whether the reader could have read it anyway.
/// A locale we don't know answers yes: staying quiet beats a wrong offer.
fn writes(lang: &str, script: Script) -> bool {
    match lang {
        "ru" | "uk" => script == Script::Cyrillic,
        // Serbia reads both of its alphabets.
        "sr" => matches!(script, Script::Cyrillic | Script::Latin),
        // Kanji-only Japanese reads as Mandarin to a script detector.
        "ja" => matches!(
            script,
            Script::Hiragana | Script::Katakana | Script::Mandarin
        ),
        "zh" => script == Script::Mandarin,
        "ko" => script == Script::Hangul,
        "ar" => script == Script::Arabic,
        "he" => script == Script::Hebrew,
        "hi" => script == Script::Devanagari,
        "cs" | "de" | "en" | "es" | "fr" | "it" | "nl" | "pl" | "pt" | "sv" | "tr" => {
            script == Script::Latin
        }
        _ => true,
    }
}

/// Collapse whitespace, drop URLs and addresses (Latin noise that would pull a
/// link-heavy newsletter towards English), and cut to the sample size.
fn sample(text: &str) -> String {
    let mut out = String::with_capacity(SAMPLE_CHARS);
    for word in text.split_whitespace() {
        if word.contains("://") || word.contains('@') || word.starts_with("www.") {
            continue;
        }
        if out.chars().count() + word.chars().count() + 1 > SAMPLE_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

fn letters(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}

/// whatlang speaks ISO 639-**3** (`rus`), the `locale` setting speaks ISO
/// 639-**1** (`ru`). Compared naively the user's own language would never match
/// and the translate bar would appear on every message, so the mapping is
/// explicit and guarded by a test.
fn iso1(lang: Lang) -> Option<&'static str> {
    Some(match lang {
        Lang::Eng => "en",
        Lang::Rus => "ru",
        // Latin-script BCS. Skim's own Serbian is the closest thing it speaks,
        // and the guess only decides *whether* to offer — the language
        // translated into always comes from the locale.
        Lang::Srp | Lang::Hrv => "sr",
        Lang::Fra => "fr",
        Lang::Deu => "de",
        Lang::Spa => "es",
        Lang::Ita => "it",
        Lang::Pol => "pl",
        Lang::Cmn => "zh",
        Lang::Jpn => "ja",
        Lang::Kor => "ko",
        Lang::Nld => "nl",
        Lang::Por => "pt",
        Lang::Tur => "tr",
        Lang::Ukr => "uk",
        Lang::Ces => "cs",
        Lang::Swe => "sv",
        Lang::Ara => "ar",
        Lang::Heb => "he",
        Lang::Hin => "hi",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RU: &str = "Здравствуйте! Напоминаем, что ваш заказ уже собран и ожидает \
        в пункте выдачи на улице Ленина. Забрать его можно в любой день до конца недели, \
        мы работаем с десяти утра до девяти вечера без перерывов и выходных.";
    const DE: &str = "Guten Tag, vielen Dank für Ihre Anfrage. Wir haben Ihre Unterlagen \
        erhalten und prüfen sie derzeit. Sobald die Prüfung abgeschlossen ist, melden wir \
        uns wieder bei Ihnen und schicken Ihnen den Vertrag zur Unterschrift zu.";
    const JA: &str =
        "お問い合わせいただきありがとうございます。ご注文の商品は本日発送いたしました。\
        配送状況は追跡番号からご確認いただけます。到着までしばらくお待ちください。\
        今後ともよろしくお願いいたします。";
    /// Serbian as half the country writes it: Latin letters, not Cyrillic.
    const SR_LATIN: &str = "Poštovani, obaveštavamo vas da je vaša pošiljka stigla u naše \
        skladište i da je možete preuzeti svakog radnog dana od osam do šesnaest časova. \
        Molimo vas da ponesete ličnu kartu i broj porudžbine.";
    /// A language deliberately outside `ALLOWED` — whatlang files it under a
    /// neighbour (Spanish, as it happens) and is sure of it.
    const HU: &str = "Tisztelt Ügyfelünk! Tájékoztatjuk, hogy a számlája elkészült és \
        letölthető a fiókjából. Kérjük, a befizetést a hónap végéig rendezze, hogy a \
        szolgáltatás megszakítás nélkül működjön tovább.";
    /// The kind of body no trigram can name: several languages at once. This is
    /// where the guess comes back unreliable and only the script is left.
    const MIXED: &str = "Dear partner, prosimo za potrditev, molimo potvrdite, bitte \
        bestätigen Sie den Termin, merci de confirmer le rendez-vous, grazie per la \
        conferma, hvala unaprijed na odgovoru.";

    const EN: &str = "Hi there, check out this week's top candidate picks, chosen from the \
        preferences you saved earlier. Each profile below has a short introduction written by \
        the candidate, and you can reply directly if one of them looks like a good match.";
    /// A stylesheet the way it arrives when a sender flattens their own HTML
    /// into the text part: no line breaks, `<style>` contents and all.
    const CSS: &str = ".bio{margin:auto}.bio .avatar{margin:auto 30px;width:fit-content}\
        @media (max-width: 991px){.bio .avatar{margin:0 auto !important}}\
        .bio .avatar img{border-radius:50%;width:100px}.bio .name{font-size:20px;\
        font-weight:600;text-align:center}.bio .location{color:#666;font-size:14px}\
        .mail-container{margin:0 auto;max-width:600px;padding:24px}\
        .request{display:block;margin-top:16px}.request .btn{background:#f26625;\
        border-radius:4px;color:#fff;display:inline-block;padding:12px 24px;\
        text-decoration:none}td{font-size:14px;line-height:20px}body{margin:0;padding:0}";

    /// The language, when the guess was solid enough to name one.
    fn code(text: &str) -> Option<&'static str> {
        match detect(text) {
            Some(Detected::Lang(code)) => Some(code),
            _ => None,
        }
    }

    #[test]
    fn detects_the_language_under_a_flattened_stylesheet() {
        // The regression: 72% of the sample was CSS, whatlang called the guess
        // unreliable, and the pane offered no translation for plain English.
        assert_eq!(code(&format!("{CSS}{EN}")), Some("en"));
    }

    #[test]
    fn a_stylesheet_is_not_a_language() {
        assert!(detect(CSS).is_none());
        assert!(!is_foreign(CSS, "ru"));
    }

    #[test]
    fn css_does_not_outvote_the_prose() {
        assert_eq!(code(&format!("{CSS}{RU}")), Some("ru"));
    }

    #[test]
    fn maps_detections_to_iso_639_1() {
        // The guard: whatlang would say "rus"/"deu"/"jpn" here, and the locale
        // setting says "ru"/"de"/"ja".
        assert_eq!(code(RU), Some("ru"));
        assert_eq!(code(DE), Some("de"));
        assert_eq!(code(JA), Some("ja"));
    }

    #[test]
    fn every_allowed_language_has_an_iso1_code() {
        for lang in ALLOWED {
            assert!(iso1(*lang).is_some(), "no ISO 639-1 code for {lang:?}");
        }
    }

    #[test]
    fn too_short_to_guess() {
        assert!(detect("Guten Tag, danke schön!").is_none());
        assert!(detect("").is_none());
    }

    #[test]
    fn links_and_numbers_alone_say_nothing() {
        let body = "https://example.com/a/b/c?utm_source=newsletter&utm_medium=email \
            no-reply@example.com 1234 5678 90 +49 30 123456 https://example.com/unsubscribe";
        assert!(detect(body).is_none());
    }

    #[test]
    fn falls_back_to_the_raw_head_for_a_bottom_posted_reply() {
        // `strip_quoted` leaves nothing here, and this is precisely the mail a
        // translation is wanted for.
        let body = format!(
            "On Tue, Aug 4, 2026 at 10:00, Ann <ann@example.com> wrote:\n\
             > What is the status?\n\n{DE}"
        );
        assert_eq!(code(&body), Some("de"));
    }

    #[test]
    fn prefers_the_senders_own_words_over_the_quoted_tail() {
        let body = format!("{DE}\n\nOn Tue, Aug 4, 2026, Ann wrote:\n> {RU}");
        assert_eq!(code(&body), Some("de"));
    }

    #[test]
    fn serbian_written_in_latin_is_still_serbian() {
        // whatlang knows Serbian only in Cyrillic; without `Hrv` in the
        // allowlist this body had no right answer and the pane stayed silent.
        assert_eq!(code(SR_LATIN), Some("sr"));
        assert!(is_foreign(SR_LATIN, "ru"));
        assert!(!is_foreign(SR_LATIN, "sr"));
    }

    #[test]
    fn a_language_outside_the_list_is_still_offered() {
        // Hungarian gets misfiled as a neighbour, which costs nothing: the
        // answer to "can this reader read it" is no either way.
        assert!(is_foreign(HU, "ru"));
        assert!(is_foreign(HU, "en"));
    }

    #[test]
    fn a_script_the_reader_does_not_use_is_offered_anyway() {
        // Nothing here is nameable, but a reader of Russian is no better off
        // for that — while a reader of English might well cope.
        assert!(is_foreign(MIXED, "ru"));
        assert!(!is_foreign(MIXED, "en"));
        assert!(is_foreign(EN, "ja"));
    }

    #[test]
    fn the_readers_own_language_is_never_offered() {
        assert!(!is_foreign(RU, "ru"));
        assert!(!is_foreign(EN, "en-US"));
        assert!(!is_foreign(DE, "de"));
        assert!(!is_foreign(JA, "ja"));
    }

    #[test]
    fn the_script_rule_stays_quiet_for_a_locale_it_does_not_know() {
        // A named language still gets compared, but a script we can't place a
        // locale in is no reason to guess on the reader's behalf.
        assert!(!is_foreign(MIXED, "qq"));
    }
}
