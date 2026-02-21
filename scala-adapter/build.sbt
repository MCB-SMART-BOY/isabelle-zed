name := "isabelle-scala-adapter"

version := "0.1.0"

scalaVersion := "3.4.0"

libraryDependencies ++= Seq(
  "org.json4s" %% "json4s-native" % "4.0.6",
  "org.json4s" %% "json4s-jackson" % "4.0.6",
  "org.scalatest" %% "scalatest" % "3.2.18" % Test,
  "org.scalatestplus" %% "mockito-4-6" % "3.2.18.0" % Test
)

scalacOptions ++= Seq(
  "-deprecation",
  "-feature",
  "-unchecked"
)

Test / logBuffered := false
