package isabelle.adapter

import ProtocolModel.*

import java.io.BufferedReader
import java.io.InputStream
import java.io.InputStreamReader
import java.io.OutputStream
import java.io.OutputStreamWriter
import java.io.PrintWriter
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.URI
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import java.util.Comparator
import java.util.concurrent.Executors
import scala.collection.concurrent.TrieMap
import scala.collection.mutable.ListBuffer
import scala.concurrent.Await
import scala.concurrent.ExecutionContext
import scala.concurrent.Future
import scala.concurrent.blocking
import scala.concurrent.duration.DurationInt
import scala.sys.process.Process
import scala.sys.process.ProcessLogger
import scala.util.Failure
import scala.util.Success
import scala.util.Try
import scala.util.matching.Regex

object AdapterMain {
  private val ErrorLineRegex: Regex =
    raw"^\*\*\* .* \(line ([0-9]+) of \"([^\"]+)\"\):\s*(.*)$$".r

  private[adapter] def parseProcessTheoriesDiagnostics(
    uri: String,
    output: String,
    exitCode: Int
  ): List[Diagnostic] = {
    if (exitCode == 0) {
      Nil
    } else {
      val lines = output.linesIterator.toList

      val parsed = lines.collect {
        case ErrorLineRegex(lineRaw, _, message) =>
          val line = Try(lineRaw.toInt).toOption.getOrElse(1)
          val safeLine = if (line < 1) 1 else line

          Diagnostic(
            uri = uri,
            range = Range(
              start = Position(safeLine, 0),
              end = Position(safeLine, 1)
            ),
            severity = "error",
            message = message.trim
          )
      }

      if (parsed.nonEmpty) {
        parsed
      } else {
        val fallbackMessage =
          lines.find(_.startsWith("*** ")).map(_.stripPrefix("*** ").trim).getOrElse {
            s"Isabelle check failed (exit code $exitCode)"
          }

        List(
          Diagnostic(
            uri = uri,
            range = Range(Position(1, 0), Position(1, 1)),
            severity = "error",
            message = fallbackMessage
          )
        )
      }
    }
  }

  final case class Config(
    mock: Boolean,
    socket: Option[(String, Int)],
    isabellePath: String,
    logic: String
  )

  trait PideBackend {
    def updateDocument(uri: String, text: String, version: Int): Future[List[Diagnostic]]
    def checkDocument(uri: String, version: Int): Future[List[Diagnostic]]
    def hover(uri: String, offset: Position): Future[String]
  }

  /**
    * Deterministic backend used for CI and local development.
    */
  final class MockPideBackend(using ExecutionContext) extends PideBackend {
    override def updateDocument(uri: String, text: String, version: Int): Future[List[Diagnostic]] =
      Future.successful(mockDiagnostics(uri))

    override def checkDocument(uri: String, version: Int): Future[List[Diagnostic]] =
      Future.successful(mockDiagnostics(uri))

    override def hover(uri: String, offset: Position): Future[String] =
      Future.successful(s"Mock hover info at ${offset.line}:${offset.col}")

    private def mockDiagnostics(uri: String): List[Diagnostic] =
      List(
        Diagnostic(
          uri = uri,
          range = Range(start = Position(1, 0), end = Position(1, 6)),
          severity = "error",
          message = "Parse error"
        )
      )
  }

  /**
    * Isabelle backend using `isabelle process_theories` for real checks.
    *
    * This is not yet full in-process PIDE integration, but it provides real
    * Isabelle diagnostics by checking pushed theory text in a temporary adhoc
    * session context.
    */
  final class IsabellePideBackend(isabellePath: String, logic: String)(using
    ExecutionContext
  ) extends PideBackend {
    private val latestDocuments = TrieMap.empty[String, (String, Int)]

    override def updateDocument(uri: String, text: String, version: Int): Future[List[Diagnostic]] =
      Future {
        latestDocuments.put(uri, (text, version))
        runIsabelleCheck(uri, text)
      }

    override def checkDocument(uri: String, version: Int): Future[List[Diagnostic]] =
      Future {
        val text = latestDocuments
          .get(uri)
          .map(_._1)
          .orElse(readDocumentFromUri(uri))
          .getOrElse("")

        if (text.trim.isEmpty) {
          List(
            Diagnostic(
              uri = uri,
              range = Range(Position(1, 0), Position(1, 1)),
              severity = "warning",
              message = "No theory text available for check"
            )
          )
        } else {
          runIsabelleCheck(uri, text)
        }
      }

    override def hover(uri: String, offset: Position): Future[String] =
      Future.successful(
        s"Hover from process_theories backend is not available yet (${offset.line}:${offset.col})"
      )

    private def runIsabelleCheck(uri: String, text: String): List[Diagnostic] =
      blocking {
        val theoryName = resolveTheoryName(uri, text)
        val tempDir = Files.createTempDirectory("isabelle-adapter-")

        try {
          val theoryFile = tempDir.resolve(s"$theoryName.thy")
          Files.writeString(theoryFile, text, StandardCharsets.UTF_8)

          val command = List(
            isabellePath,
            "process_theories",
            "-l",
            logic,
            "-D",
            tempDir.toString,
            "-O",
            theoryName
          )

          val output = new StringBuilder
          val logger = ProcessLogger(
            out => output.append(out).append('\n'),
            err => output.append(err).append('\n')
          )

          val exitCode = Process(command).!(logger)
          parseProcessTheoriesDiagnostics(uri, output.toString, exitCode)
        } finally {
          deleteRecursively(tempDir)
        }
      }

    private def resolveTheoryName(uri: String, text: String): String =
      extractTheoryName(text)
        .orElse(theoryNameFromUri(uri))
        .map(_.replaceAll("[^A-Za-z0-9_'.-]", "_"))
        .filter(_.nonEmpty)
        .getOrElse("Scratch")

    private def extractTheoryName(text: String): Option[String] = {
      val regex = raw"(?m)^\s*theory\s+([A-Za-z0-9_'.-]+)\b".r
      regex.findFirstMatchIn(text).map(_.group(1))
    }

    private def theoryNameFromUri(uri: String): Option[String] =
      Try {
        val parsed = URI.create(uri)
        if (parsed.getScheme == "file") {
          val name = Paths.get(parsed).getFileName.toString
          if (name.endsWith(".thy")) Some(name.stripSuffix(".thy")) else None
        } else {
          None
        }
      }.toOption.flatten

    private def readDocumentFromUri(uri: String): Option[String] =
      Try {
        val parsed = URI.create(uri)
        if (parsed.getScheme == "file") {
          val path = Paths.get(parsed)
          if (Files.isRegularFile(path)) {
            Some(Files.readString(path, StandardCharsets.UTF_8))
          } else {
            None
          }
        } else {
          None
        }
      }.toOption.flatten

    private def deleteRecursively(path: Path): Unit = {
      if (!Files.exists(path)) {
        return
      }

      val stream = Files.walk(path)
      try {
        stream
          .sorted(Comparator.reverseOrder())
          .forEach { p =>
            Files.deleteIfExists(p)
            ()
          }
      } finally {
        stream.close()
      }
    }
  }

  final class AdapterService(backend: PideBackend)(using ec: ExecutionContext) {
    private val writeLock = new AnyRef

    def processLine(line: String, writer: PrintWriter): Future[Unit] = {
      decodeEnvelope(line) match {
        case Left(error) =>
          System.err.println(s"Invalid NDJSON message: $error")
          Future.unit
        case Right(envelope) =>
          decodeIncoming(envelope) match {
            case Left(error) =>
              System.err.println(s"Unsupported message payload: $error")
              Future.unit
            case Right(DocumentPushRequest(env, payload)) =>
              backend
                .updateDocument(payload.uri, payload.text, env.version)
                .map(diagnostics => diagnosticsResponse(env, diagnostics))
                .map(writeEnvelope(_, writer))
            case Right(DocumentCheckRequest(env, payload)) =>
              backend
                .checkDocument(payload.uri, payload.version)
                .map(diagnostics => diagnosticsResponse(env, diagnostics))
                .map(writeEnvelope(_, writer))
            case Right(MarkupRequest(env, payload)) =>
              backend
                .hover(payload.uri, payload.offset)
                .map(info => markupResponse(env, payload.uri, payload.offset, info))
                .map(writeEnvelope(_, writer))
          }
      }
    }

    private def writeEnvelope(envelope: Envelope, writer: PrintWriter): Unit =
      writeLock.synchronized {
        writer.println(encodeEnvelope(envelope))
        writer.flush()
      }
  }

  private val workerPool = Executors.newFixedThreadPool(4)
  given ExecutionContext = ExecutionContext.fromExecutor(workerPool)

  def main(args: Array[String]): Unit = {
    val config = parseArgs(args)
    val backend: PideBackend =
      if (config.mock) new MockPideBackend()
      else new IsabellePideBackend(config.isabellePath, config.logic)

    val service = new AdapterService(backend)

    config.socket match {
      case Some((host, port)) => runSocketServer(host, port, service)
      case None               => runStdio(service)
    }
  }

  def parseArgs(args: Array[String]): Config = {
    val default = Config(mock = false, socket = None, isabellePath = "isabelle", logic = "HOL")

    args.foldLeft(default) { (config, arg) =>
      if (arg == "--mock") {
        config.copy(mock = true)
      } else if (arg.startsWith("--socket=")) {
        val socket = parseSocketArg(arg.stripPrefix("--socket="))
        config.copy(socket = socket)
      } else if (arg.startsWith("--isabelle-path=")) {
        config.copy(isabellePath = arg.stripPrefix("--isabelle-path="))
      } else if (arg.startsWith("--logic=")) {
        config.copy(logic = arg.stripPrefix("--logic="))
      } else {
        config
      }
    }
  }

  def runWithStreams(config: Config, input: InputStream, output: OutputStream): Unit = {
    val backend: PideBackend =
      if (config.mock) new MockPideBackend()
      else new IsabellePideBackend(config.isabellePath, config.logic)

    val service = new AdapterService(backend)
    val reader = new BufferedReader(new InputStreamReader(input, StandardCharsets.UTF_8))
    val writer = new PrintWriter(new OutputStreamWriter(output, StandardCharsets.UTF_8), false)

    streamLoop(reader, writer, service)
  }

  private def runStdio(service: AdapterService): Unit = {
    val reader = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))
    val writer = new PrintWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8), false)
    streamLoop(reader, writer, service)
  }

  private def runSocketServer(host: String, port: Int, service: AdapterService): Unit = {
    val server = new ServerSocket()
    server.bind(InetSocketAddress(host, port))
    System.err.println(s"Adapter listening on $host:$port")

    while (true) {
      val socket = server.accept()
      Future {
        val reader = new BufferedReader(
          new InputStreamReader(socket.getInputStream, StandardCharsets.UTF_8)
        )
        val writer = new PrintWriter(
          new OutputStreamWriter(socket.getOutputStream, StandardCharsets.UTF_8),
          false
        )
        try {
          streamLoop(reader, writer, service)
        } finally {
          socket.close()
        }
      }
    }
  }

  private def streamLoop(reader: BufferedReader, writer: PrintWriter, service: AdapterService): Unit = {
    val inflight = ListBuffer.empty[Future[Unit]]
    var line = reader.readLine()
    while (line != null) {
      val pending = service.processLine(line, writer)
      inflight += pending
      pending.onComplete {
        case Success(_) => ()
        case Failure(error) =>
          System.err.println(s"Request processing failed: ${error.getMessage}")
      }
      line = reader.readLine()
    }

    Await.result(Future.sequence(inflight.toList), 2.seconds)
  }

  private def parseSocketArg(value: String): Option[(String, Int)] =
    value.split(':').toList match {
      case host :: port :: Nil =>
        Try(port.toInt).toOption.map(p => (host, p))
      case _ =>
        System.err.println(s"Invalid --socket argument: $value")
        None
    }
}
