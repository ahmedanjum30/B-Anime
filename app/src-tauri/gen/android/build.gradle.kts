buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:9.3.1")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.2.10")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
    // The Kotlin compiled here is Tauri's library, its plugins, and files Tauri regenerates on
    // every build (app/.../generated); none can be edited, and their deprecation warnings are
    // Tauri's to fix. Our own Kotlin is MainActivity alone. Re-enable when adding real Kotlin.
    tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
        compilerOptions.suppressWarnings.set(true)
    }
}

tasks.register("clean").configure {
    delete("build")
}

