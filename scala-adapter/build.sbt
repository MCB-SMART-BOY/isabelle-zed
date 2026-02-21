name := "isabelle-scala-adapter"

version := "0.1.0"

scalaVersion := "3.4.2"

libraryDependencies ++= Seq(
  "io.circe" %% "circe-core" % "0.14.10",
  "io.circe" %% "circe-generic" % "0.14.10",
  "io.circe" %% "circe-parser" % "0.14.10",
  "org.scalatest" %% "scalatest" % "3.2.19" % Test
)

Compile / run / fork := true
Test / fork := false
Test / parallelExecution := false

scalacOptions ++= Seq(
  "-deprecation",
  "-feature",
  "-unchecked",
  "-Xfatal-warnings"
)
