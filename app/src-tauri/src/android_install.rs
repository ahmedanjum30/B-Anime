//! Android: hands a downloaded, checksum-verified APK to the system package installer, and posts
//! the "update ready" notification whose tap does the same.
//!
//! The APK sits in the app's cache folder, shared through the manifest's FileProvider
//! (`<cache-path>` in res/xml/file_paths.xml). Android shows its own "update this app?" screen; it
//! asks first to allow installs from B-Anime (REQUEST_INSTALL_PACKAGES in the manifest).

use std::path::Path;

use jni::objects::{JClass, JObject, JValue};
use jni::{JNIEnv, JavaVM};

/// Intent flags: FLAG_GRANT_READ_URI_PERMISSION | FLAG_ACTIVITY_NEW_TASK.
const FLAGS: i32 = 0x0000_0001 | 0x1000_0000;
/// PendingIntent flags: FLAG_IMMUTABLE | FLAG_UPDATE_CURRENT.
const PENDING_FLAGS: i32 = 0x0400_0000 | 0x0800_0000;
/// The notification's id (one "update ready" at a time) and channel.
const NOTIFICATION_ID: i32 = 4401;
const CHANNEL_ID: &str = "updates";
/// NotificationManager.IMPORTANCE_DEFAULT: shows in the shade and the status bar, with sound.
const IMPORTANCE_DEFAULT: i32 = 3;

/// Runs `f` with the JVM attached and the app's context, clearing any Java exception after.
fn with_env<T>(what: &str, f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>) -> Result<T, String> {
    let ctx = ndk_context::android_context();
    // SAFETY: ndk-context holds the process's JavaVM and the activity, set up by Tauri at launch.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    // SAFETY: a global reference owned by ndk-context; never deleted here.
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };
    let result = f(&mut env, &context);
    // A Java exception must be cleared before this thread talks to the JVM again.
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result.map_err(|e| format!("{what}: {e}"))
}

/// The intent that opens the system installer on the APK.
fn installer_intent<'a>(env: &mut JNIEnv<'a>, context: &JObject, apk: &Path, authority: &str) -> jni::errors::Result<JObject<'a>> {
    let path = env.new_string(apk.to_string_lossy())?;
    let file = env.new_object("java/io/File", "(Ljava/lang/String;)V", &[JValue::Object(&path)])?;

    // FileProvider is an app class: load it through the app's class loader (this thread's
    // default loader only knows framework classes).
    let loader = env.call_method(context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
    let name = env.new_string("androidx.core.content.FileProvider")?;
    let provider: JClass = env
        .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
        .l()?
        .into();
    let authority = env.new_string(authority)?;
    let uri = env
        .call_static_method(
            &provider,
            "getUriForFile",
            "(Landroid/content/Context;Ljava/lang/String;Ljava/io/File;)Landroid/net/Uri;",
            &[JValue::Object(context), JValue::Object(&authority), JValue::Object(&file)],
        )?
        .l()?;

    let action = env.new_string("android.intent.action.VIEW")?;
    let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
    let mime = env.new_string("application/vnd.android.package-archive")?;
    env.call_method(
        &intent,
        "setDataAndType",
        "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&uri), JValue::Object(&mime)],
    )?;
    env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(FLAGS)])?;
    Ok(intent)
}

pub fn install_apk(apk: &Path, authority: &str) -> Result<(), String> {
    with_env("couldn't open the Android installer", |env, context| {
        let intent = installer_intent(env, context, apk, authority)?;
        env.call_method(context, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    })
}

fn sdk_int(env: &mut JNIEnv) -> jni::errors::Result<i32> {
    env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()
}

/// Posts "B-Anime X is ready · Tap to install": a tap opens the system installer on the APK,
/// whether or not B-Anime is still open. Android 13 and later ask once for permission to show
/// notifications (the first time an update is ready); without it, only the in-app notice shows.
pub fn notify_update_ready(apk: &Path, authority: &str, title: &str, text: &str) -> Result<(), String> {
    with_env("couldn't post the update notification", |env, context| {
        let sdk = sdk_int(env)?;
        if sdk >= 33 {
            let permission = env.new_string("android.permission.POST_NOTIFICATIONS")?;
            let state = env
                .call_method(context, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&permission)])?
                .i()?;
            if state != 0 {
                // The context is the activity: ask (the answer applies to the next update).
                let list = env.new_object_array(1, "java/lang/String", &permission)?;
                let _ = env.call_method(context, "requestPermissions", "([Ljava/lang/String;I)V", &[
                    JValue::Object(&list),
                    JValue::Int(NOTIFICATION_ID),
                ]);
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
            }
        }

        let service = env.new_string("notification")?;
        let manager = env
            .call_method(context, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&service)])?
            .l()?;
        let channel_id = env.new_string(CHANNEL_ID)?;
        if sdk >= 26 {
            let name = env.new_string("Updates")?;
            let channel = env.new_object(
                "android/app/NotificationChannel",
                "(Ljava/lang/String;Ljava/lang/CharSequence;I)V",
                &[JValue::Object(&channel_id), JValue::Object(&name), JValue::Int(IMPORTANCE_DEFAULT)],
            )?;
            env.call_method(&manager, "createNotificationChannel", "(Landroid/app/NotificationChannel;)V", &[JValue::Object(&channel)])?;
        }

        let intent = installer_intent(env, context, apk, authority)?;
        let pending = env
            .call_static_method(
                "android/app/PendingIntent",
                "getActivity",
                "(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
                &[JValue::Object(context), JValue::Int(NOTIFICATION_ID), JValue::Object(&intent), JValue::Int(PENDING_FLAGS)],
            )?
            .l()?;

        // The app's own icon as the small icon.
        let info = env.call_method(context, "getApplicationInfo", "()Landroid/content/pm/ApplicationInfo;", &[])?.l()?;
        let icon = env.get_field(&info, "icon", "I")?.i()?;

        let builder = if sdk >= 26 {
            env.new_object(
                "android/app/Notification$Builder",
                "(Landroid/content/Context;Ljava/lang/String;)V",
                &[JValue::Object(context), JValue::Object(&channel_id)],
            )?
        } else {
            env.new_object("android/app/Notification$Builder", "(Landroid/content/Context;)V", &[JValue::Object(context)])?
        };
        let title = env.new_string(title)?;
        let text = env.new_string(text)?;
        let b = "Landroid/app/Notification$Builder;";
        env.call_method(&builder, "setSmallIcon", format!("(I){b}"), &[JValue::Int(icon)])?;
        env.call_method(&builder, "setContentTitle", format!("(Ljava/lang/CharSequence;){b}"), &[JValue::Object(&title)])?;
        env.call_method(&builder, "setContentText", format!("(Ljava/lang/CharSequence;){b}"), &[JValue::Object(&text)])?;
        env.call_method(&builder, "setContentIntent", format!("(Landroid/app/PendingIntent;){b}"), &[JValue::Object(&pending)])?;
        env.call_method(&builder, "setAutoCancel", format!("(Z){b}"), &[JValue::Bool(1)])?;
        let notification = env.call_method(&builder, "build", "()Landroid/app/Notification;", &[])?.l()?;
        env.call_method(&manager, "notify", "(ILandroid/app/Notification;)V", &[JValue::Int(NOTIFICATION_ID), JValue::Object(&notification)])?;
        Ok(())
    })
}
