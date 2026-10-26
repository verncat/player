use jni::{
    objects::{JObject, JString},
    sys::jstring,
    JNIEnv,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

static CALLS: AtomicU64 = AtomicU64::new(0);

fn call(method: &str, args: &str) -> Result<Value, String> {
    let args: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
    match method {
        "echo" => Ok(json!({
            "backend":"Rust via JNI", "arch":std::env::consts::ARCH,
            "message":args.get("message").and_then(Value::as_str).ok_or("message must be a string")?,
            "callCount":CALLS.fetch_add(1, Ordering::Relaxed) + 1,
        })),
        _ => Err("METHOD_NOT_FOUND".into()),
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_verncat_player_example_nativeplugin_NativePlugin_nativeCall(
    mut env: JNIEnv,
    _this: JObject,
    method: JString,
    args: JString,
) -> jstring {
    // Never unwind across JNI. Native plugins must follow the same rule.
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value, String> {
            let method: String = env.get_string(&method).map_err(|e| e.to_string())?.into();
            let args: String = env.get_string(&args).map_err(|e| e.to_string())?.into();
            call(&method, &args)
        }))
        .unwrap_or_else(|_| Err("Native panic".into()));
    let response = match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    match env.new_string(response.to_string()) {
        Ok(value) => value.into_raw(),
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                "Could not allocate JNI response",
            );
            std::ptr::null_mut()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn echo_preserves_unicode_and_rejects_bad_requests() {
        let result = call("echo", r#"{"message":"Привет 🎵 \"Rust\""}"#).unwrap();
        assert_eq!(result["message"], "Привет 🎵 \"Rust\"");
        assert_eq!(result["backend"], "Rust via JNI");
        assert!(result["callCount"].as_u64().unwrap() > 0);
        assert!(call("echo", "{}").is_err());
        assert!(call("echo", "invalid json").is_err());
        assert_eq!(call("unknown", "{}").unwrap_err(), "METHOD_NOT_FOUND");
    }
}
