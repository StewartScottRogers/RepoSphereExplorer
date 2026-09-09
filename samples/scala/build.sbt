ThisBuild / organization := "com.example"
ThisBuild / version      := "0.8.0"
ThisBuild / scalaVersion := "3.6.4"

lazy val root = (project in file("."))
  .settings(
    name        := "event-stream",
    description := "An append-only event store with projections.",
    licenses    := Seq("MIT" -> url("https://opensource.org/licenses/MIT")),
    scalacOptions ++= Seq(
      "-deprecation",
      "-feature",
      "-unchecked",
      "-Werror",
      "-Wunused:all",
      "-source:future"
    ),
    libraryDependencies ++= Seq(
      "org.scalameta" %% "munit" % "1.1.0" % Test
    ),
    Test / parallelExecution := true
  )
