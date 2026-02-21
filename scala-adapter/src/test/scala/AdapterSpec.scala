package isabelle.adapter

import ProtocolModel.*

import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.PipedInputStream
import java.io.PipedOutputStream
import java.io.PrintWriter
import scala.concurrent.Await
import scala.concurrent.Future
import scala.concurrent.duration.DurationInt
import scala.concurrent.ExecutionContext.Implicits.global
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers

class AdapterSpec extends AnyFlatSpec with Matchers {

  "ProtocolModel" should "parse the canonical document.push and diagnostics examples" in {
    val push = decodeEnvelope(DocumentPushExample)
    push.isRight shouldBe true

    val diagnostics = decodeEnvelope(DiagnosticsExample)
    diagnostics.isRight shouldBe true
    diagnostics.toOption.get.`type` shouldBe "diagnostics"
  }

  "AdapterMain" should "run in --mock mode and return diagnostics for document.push" in {
    val adapterInput = new PipedInputStream()
    val clientWriterStream = new PipedOutputStream(adapterInput)

    val adapterOutput = new PipedOutputStream()
    val clientReaderStream = new PipedInputStream(adapterOutput)

    val runner = Future {
      AdapterMain.runWithStreams(
        config = AdapterMain.parseArgs(Array("--mock")),
        input = adapterInput,
        output = adapterOutput
      )
    }

    val clientWriter = new PrintWriter(clientWriterStream, true)
    val clientReader = new BufferedReader(new InputStreamReader(clientReaderStream))

    clientWriter.println(DocumentPushExample)
    clientWriter.flush()
    clientWriter.close()

    val responseLine = Await.result(Future(clientReader.readLine()), 1.second)
    responseLine should not be null

    val response = decodeEnvelope(responseLine)
    response.isRight shouldBe true

    val envelope = response.toOption.get
    envelope.`type` shouldBe "diagnostics"
    envelope.id shouldBe "msg-0001"
    envelope.session shouldBe "s1"
    envelope.version shouldBe 1

    val diagnostics = envelope.payload.as[List[Diagnostic]]
    diagnostics.isRight shouldBe true
    diagnostics.toOption.get should have size 1
    diagnostics.toOption.get.head.message shouldBe "Parse error"

    Await.result(runner, 1.second)
  }

  it should "parse --logic argument" in {
    val config = AdapterMain.parseArgs(Array("--logic=HOL", "--mock"))
    config.logic shouldBe "HOL"
    config.mock shouldBe true
  }

  "parseProcessTheoriesDiagnostics" should "extract line-based errors from process_theories output" in {
    val output =
      """Running Draft ...
        |Draft FAILED (see also "isabelle build_log -H Error Draft")
        |*** Outer syntax error (line 5 of "/tmp/Broken.thy"): proposition expected,
        |*** but end-of-input (line 5 of "/tmp/Broken.thy") was found
        |*** At command "<malformed>" (line 5 of "/tmp/Broken.thy")
        |Unfinished session(s): Draft
        |""".stripMargin

    val diagnostics =
      AdapterMain.parseProcessTheoriesDiagnostics(
        uri = "file:///tmp/Broken.thy",
        output = output,
        exitCode = 1
      )

    diagnostics should not be empty
    diagnostics.head.uri shouldBe "file:///tmp/Broken.thy"
    diagnostics.head.range.start.line shouldBe 5
    diagnostics.head.severity shouldBe "error"
  }
}
