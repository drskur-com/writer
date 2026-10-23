//! 확장 그림 검토(ADR-0030): `run_extension`이 그린 그림을 대화 맥락 없는 모델이 따로 보고 결함과 점수를 매긴다.
//! 그린 모델은 자기 그림에 관대해서, 엉망인 그림도 "됐다"고 내놓기 쉽다.
//!
//! - 검토자는 사용자 요청, 그린 모델이 적은 목표(`goal`), 그림만 본다.
//! - 답이 끝날 때 초안의 마지막 검토가 [`PASS_SCORE`]보다 낮거나, 고친 뒤 다시 그려 보지 않았으면 모델에게 돌려보낸다
//!   ([`MAX_SEND_BACKS`]번까지). 그 뒤에는 그대로 승인을 묻는다.
//!
//! 글은 모두 모델이 읽으므로 영어다.

/// 이 점수부터 사용자에게 보여 줄 만하다고 본다.
pub const PASS_SCORE: u8 = 7;

/// 답이 끝날 때 검토를 통과하지 못한 초안을 모델에게 돌려보내는 최대 횟수.
pub const MAX_SEND_BACKS: usize = 2;

/// 검토자의 시스템 프롬프트.
pub const SYSTEM: &str = "You are a demanding art director. A drawing assistant in a writing app drew a picture with code \
(a Lua canvas API with vector paths, gradients, and flat-shaded 3D), and you review the rendered picture before the user \
sees it. You see only the picture, the user's request, and the assistant's stated goal.

Judge it as the user will see it, not by effort. Look for: wrong or awkward proportions; parts that should touch or line up \
but float, overlap wrongly, or are hidden behind other shapes; stiff, lumpy, or jagged shapes where smooth ones are needed; \
features in the wrong place (eyes, hands, joints, legs); things cut off at the canvas edge or crowded into a corner; poor \
composition or empty space; clashing or muddy colors; text that overlaps or is unreadable; and anything that does not match \
the request. For several frames of an animation, also judge whether the motion is consistent and believable.

Reply in English with at most five problems, most serious first. For each, say where it is (in canvas units, the picture \
is the canvas scaled uniformly) and give a concrete fix in drawing terms (for example \"move the left eye up about 10 and \
make it the same size as the right one\"). Do not praise and do not rewrite the code. If nothing is seriously wrong, say \
what would make it better.

End with one line `SCORE: n`, where n is 1 to 10: 10 is a polished illustration, 7 is good enough to show the user \
without embarrassment, 4 is recognizable but clearly awkward, 1 is broken or unrecognizable. Be strict: most first \
attempts deserve 3 to 5.";

/// 검토 요청 글(그림은 이 뒤에 붙인다).
pub fn request(
    user_request: &str,
    goal: &str,
    extension: &str,
    canvas: (f32, f32),
    frames: usize,
) -> String {
    let goal = if goal.trim().is_empty() {
        "(not given)"
    } else {
        goal.trim()
    };
    let frames = if frames > 1 {
        format!(
            " The picture shows {frames} frames of an animation in a grid, in order, each labeled with its time t."
        )
    } else {
        String::new()
    };
    format!(
        "The user's request:\n<request>\n{}\n</request>\n\nThe assistant's goal for this drawing:\n<goal>\n{goal}\n</goal>\n\n\
         Extension: {extension}. Canvas: {} x {} units.{frames}",
        user_request.trim(),
        canvas.0,
        canvas.1,
    )
}

/// 검토 글 마지막의 `SCORE: n`.
pub fn score(review: &str) -> Option<u8> {
    review.lines().rev().find_map(|line| {
        let rest = line
            .trim()
            .trim_matches('*')
            .trim()
            .strip_prefix("SCORE:")?
            .trim();
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u8>().ok().filter(|n| (1..=10).contains(n))
    })
}

/// 도구 결과에 붙일 검토.
pub fn report(review: &str, score: Option<u8>) -> String {
    let body = review
        .lines()
        .filter(|line| !line.trim().trim_matches('*').trim().starts_with("SCORE:"))
        .collect::<Vec<_>>()
        .join("\n");
    let verdict = match score {
        Some(n) if n >= PASS_SCORE => format!(
            "Score {n}/10 (passing is {PASS_SCORE}). Consider the remaining points if they are quick to fix."
        ),
        Some(n) => format!(
            "Score {n}/10 (passing is {PASS_SCORE}). Fix these problems and run_extension again before you finish; \
             if you finish below {PASS_SCORE}, the app sends the extension back to you instead of offering it to the user."
        ),
        None => "The reviewer gave no score.".into(),
    };
    format!(
        "\n\nIndependent review of the picture (by a reviewer who sees only the request, your goal, and the picture):\n{}\n{verdict}",
        body.trim()
    )
}

/// 답을 끝내려는 모델에게 돌려보낼 글. 검토를 통과하지 못한 초안이 없으면 `None`.
pub fn send_back(problems: &[String]) -> Option<String> {
    if problems.is_empty() {
        return None;
    }
    Some(format!(
        "[From the app, not the user] Before the user is asked to approve your extension changes:\n{}\n\
         Keep improving: fix the problems, run_extension again, and finish when it passes. If you cannot do better, \
         say plainly in your reply what is still wrong.",
        problems
            .iter()
            .map(|problem| format!("- {problem}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_score_on_the_last_line() {
        assert_eq!(score("1. The head floats.\nSCORE: 4"), Some(4));
        assert_eq!(score("ok\n**SCORE: 8/10**\n"), Some(8));
        assert_eq!(score("SCORE: 11"), None);
        assert_eq!(score("no score"), None);
    }

    #[test]
    fn report_hides_the_score_line_and_asks_for_fixes_below_passing() {
        let text = report("1. The arm is detached.\nSCORE: 3", Some(3));
        assert!(!text.contains("SCORE:"));
        assert!(text.contains("The arm is detached."));
        assert!(text.contains("Score 3/10"));
        assert!(text.contains("sends the extension back"));
        assert!(!report("Fine.\nSCORE: 8", Some(8)).contains("sends"));
    }

    #[test]
    fn sends_back_only_when_something_failed() {
        assert_eq!(send_back(&[]), None);
        let text = send_back(&["Extensions/cat.lua scored 4/10.".into()]).unwrap();
        assert!(text.contains("- Extensions/cat.lua scored 4/10."));
    }
}
