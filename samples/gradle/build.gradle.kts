plugins {
    id("java-library")
    id("org.springframework.boot") version "3.2.5"
}

group = "com.example.pipeline"
version = "1.4.0"

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(21)
    }
}

repositories {
    mavenCentral()
    google()
}
