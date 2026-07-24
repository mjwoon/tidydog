//! API 키 조회 헬퍼.
//!
//! 우선순위:
//!   1. ANTHROPIC_API_KEY 환경변수 (개발자 .env 주입 / CI)
//!   2. TIDYDOG_API_KEY 환경변수 (대체 이름)
//!   3. OS 키체인 (배포된 앱 최종 사용자)
//!
//! N2 경계: env fallback은 "개발자 자기 키 주입" 용도.
//! 최종 사용자의 키는 키체인에만 보관. 어느 경로든 키를 로그/에러 메시지에 남기지 않는다.

pub fn get_api_key() -> Result<String, String> {
    get_api_key_inner("tidydog", "anthropic_api_key")
}

// 테스트에서 키체인 서비스명을 주입할 수 있도록 분리.
fn get_api_key_inner(service: &str, account: &str) -> Result<String, String> {
    // 1순위: 환경변수 (개발 .env 또는 CI secret)
    for var in &["ANTHROPIC_API_KEY", "TIDYDOG_API_KEY"] {
        if let Ok(key) = std::env::var(var) {
            let key = key.trim().to_string();
            if !key.is_empty() {
                return Ok(key);
            }
        }
    }

    // 2순위: OS 키체인 (최종 사용자)
    keyring::Entry::new(service, account)
        .map_err(|e| format!("keyring 오류: {e}"))?
        .get_password()
        .map_err(|_| {
            "API 키가 없습니다. 설정(⚙)에서 입력하거나 ANTHROPIC_API_KEY 환경변수를 설정하세요."
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // env 변수는 프로세스 전역이라 병렬 테스트 시 오염됨 — 직렬화 필수.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // 존재하지 않는 키체인 항목 — 실제 키체인을 건드리지 않고 keychain 경로 강제 실패.
    const FAKE_SVC: &str = "tidydog-test-nonexistent-xyzzy";
    const FAKE_ACC: &str = "no_such_key_xyzzy";

    fn clear_test_env() {
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::remove_var("TIDYDOG_API_KEY");
    }

    #[test]
    fn env_var_takes_priority_over_keychain() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_test_env();
        std::env::set_var("ANTHROPIC_API_KEY", "sk-test-env");
        let result = get_api_key_inner(FAKE_SVC, FAKE_ACC);
        clear_test_env();
        assert_eq!(result.unwrap(), "sk-test-env");
    }

    #[test]
    fn tidydog_api_key_is_secondary_env_fallback() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_test_env();
        std::env::set_var("TIDYDOG_API_KEY", "sk-test-tidydog");
        let result = get_api_key_inner(FAKE_SVC, FAKE_ACC);
        clear_test_env();
        assert_eq!(result.unwrap(), "sk-test-tidydog");
    }

    #[test]
    fn no_env_no_keychain_returns_api_key_error() {
        // env 둘 다 없고 키체인도 없을 때 → Err, 메시지에 "API 키" 포함 (App.tsx isApiKeyError 감지 기준).
        let _lock = ENV_LOCK.lock().unwrap();
        clear_test_env();
        let err = get_api_key_inner(FAKE_SVC, FAKE_ACC)
            .expect_err("env/키체인 둘 다 없으면 반드시 Err이어야 한다");
        clear_test_env();
        assert!(
            err.contains("API 키"),
            "App.tsx isApiKeyError가 'API 키'로 감지 — 에러 메시지에 포함 필요. got: {err}"
        );
    }
}
