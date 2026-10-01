//! Conservative, local intent filter. A positive result permits delivery only;
//! it supplies neither authority nor a replacement for correction validation.

pub(super) fn literary_input(text: &str) -> bool {
    // A quoted rendering/qualification may legitimately contain technical words.
    // Exact grammar is an intent signal; server-side selection validation remains mandatory.
    if crate::application::correction_parser::parse_correction_v1(
        &crate::application::correction_parser::OriginReceipt { text: text.into() },
    )
    .is_ok()
    {
        return true;
    }
    let lower = text.to_lowercase();
    let words: Vec<_> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    // Mixed infrastructure/story requests are uncertain and are not retained.
    if words.iter().any(|word| {
        matches!(
            *word,
            "github"
                | "gitlab"
                | "oauth"
                | "cli"
                | "systemd"
                | "cargo"
                | "rustc"
                | "issues"
                | "issue"
                | "worktree"
                | "worktrees"
                | "debug"
                | "install"
                | "uninstall"
                | "login"
                | "launcher"
                | "browser"
                | "desktop"
                | "установи"
                | "установить"
                | "установка"
                | "удали"
                | "браузер"
                | "сборка"
                | "сборки"
                | "тесты"
                | "компиляция"
                | "авторизация"
                | "логин"
                | "code"
                | "parser"
                | "css"
                | "api"
                | "typescript"
                | "svelte"
                | "код"
                | "коде"
                | "кода"
                | "парсер"
                | "парсера"
                | "хук"
                | "хука"
        )
    }) {
        return false;
    }
    words.iter().any(|word| {
        matches!(
            *word,
            "chapter"
                | "chapters"
                | "manuscript"
                | "character"
                | "characters"
                | "narrator"
                | "dialogue"
                | "plot"
                | "prose"
                | "termbase"
                | "translation"
                | "главу"
                | "главы"
                | "глава"
                | "рукопись"
                | "рукописи"
                | "персонаж"
                | "персонажа"
                | "персонажи"
                | "персонажей"
                | "рассказчик"
                | "диалог"
                | "диалоги"
                | "сюжет"
                | "сюжета"
                | "канон"
                | "канона"
                | "перевод"
                | "перевода"
                | "переведи"
                | "переводи"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_uncertain_and_mixed_requests_are_not_literary_input() {
        for text in [
            "Да, поехали",
            "Install the CLI",
            "Check browser OAuth login",
            "проверь issues проекта",
            "Install a browser to read the chapter",
            "Fix translation parser",
            "Проверь обработку канона в коде",
            "ordinary conversational text",
        ] {
            assert!(!literary_input(text), "{text}");
        }
    }

    #[test]
    fn bilingual_story_input_and_exact_corrections_are_relevant() {
        for text in [
            "Персонаж не знает тайну до третьей главы",
            "Please translate this chapter.",
            "translate this as B",
            "that memory is wrong",
            "translate \"browser\" as \"браузер\"",
            "qualify that memory as \"He uses a browser\"",
        ] {
            assert!(literary_input(text), "{text}");
        }
    }
}
