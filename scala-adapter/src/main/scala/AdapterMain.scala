package isabelle.adapter

import scala.io.StdIn
import scala.io.Source
import scala.util.{Try, Using}
import java.io.{PrintWriter, BufferedReader, InputStreamReader}
import java.net.Socket

import org.json4s._
import org.json4s.native.Serialization

object AdapterMain extends App {

  implicit val formats = org.json4s.native.Serialization.formats(NoTypeHints)

  private var mockMode = false
  private var socketHost: Option[String] = None
  private var socketPort: Option[Int] = None

  args.foreach {
    case "--mock" => mockMode = true
    case s if s.startsWith("--socket=") =>
      val value = s.drop("--socket=".length)
      if (value.contains(":")) {
        val parts = value.split(":")
        socketHost = Some(parts(0))
        socketPort = Some(parts(1).toInt)
      }
    case _ =>
  }

  if (mockMode) {
    println("Starting in mock mode...")
    runMockMode()
  } else if (socketHost.isDefined && socketPort.isDefined) {
    println(s"Connecting to socket ${socketHost.get}:${socketPort.get}...")
    runSocketMode(socketHost.get, socketPort.get)
  } else {
    println("Running in stdin/stdout mode...")
    runStdinMode()
  }

  def runStdinMode(): Unit = {
    val input = Source.fromInputStream(System.in)
    val reader = new BufferedReader(new InputStreamReader(System.in))
    
    try {
      var running = true
      while (running) {
        val line = reader.readLine()
        if (line == null) {
          running = false
        } else {
          processLine(line)
        }
      }
    } finally {
      reader.close()
    }
  }

  def runSocketMode(host: String, port: Int): Unit = {
    Using(Socket(host, port)) { socket =>
      val reader = new BufferedReader(new InputStreamReader(socket.getInputStream))
      val writer = new PrintWriter(socket.getOutputStream, true)
      
      try {
        var running = true
        while (running) {
          val line = reader.readLine()
          if (line == null) {
            running = false
          } else {
            processLine(line, Some(writer))
          }
        }
      } finally {
        reader.close()
        writer.close()
      }
    }.getOrElse {
      System.err.println("Failed to connect to socket")
    }
  }

  def runMockMode(): Unit = {
    val reader = new BufferedReader(new InputStreamReader(System.in))
    
    try {
      var running = true
      while (running) {
        val line = reader.readLine()
        if (line == null) {
          running = false
        } else {
          processLineMock(line)
        }
      }
    } finally {
      reader.close()
    }
  }

  def processLine(line: String, writer: Option[PrintWriter] = None): Unit = {
    JsonMessage.parse(line) match {
      case Some(msg) =>
        handleMessage(msg, writer)
      case None =>
        System.err.println(s"Failed to parse: $line")
    }
  }

  def processLineMock(line: String): Unit = {
    JsonMessage.parse(line) match {
      case Some(msg) =>
        handleMessageMock(msg)
      case None =>
        System.err.println(s"Failed to parse: $line")
    }
  }

  def handleMessage(msg: JsonMessage, writer: Option[PrintWriter]): Unit = {
    val msgType = MessageType.fromString(msg.`type`)
    
    msgType match {
      case DocumentPush =>
        handleDocumentPush(msg, writer)
      case DocumentCheck =>
        handleDocumentCheck(msg, writer)
      case Markup =>
        handleMarkup(msg, writer)
      case _ =>
        System.err.println(s"Unknown message type: ${msg.`type`}")
    }
  }

  def handleDocumentPush(msg: JsonMessage, writer: Option[PrintWriter]): Unit = {
    val payload = PayloadParser.extractDocumentPush(msg.payload)
    
    payload.foreach { p =>
      println(s"Document push: ${p.uri} (${p.text.length} chars)")
      
      val diagnostics = if (mockMode) {
        generateMockDiagnostics(p.uri)
      } else {
        generateIsabelleDiagnostics(p.uri, p.text)
      }
      
      val response = JsonMessage.createDiagnostics(
        session = msg.session.getOrElse("default"),
        version = msg.version.getOrElse(1L),
        diagnostics = diagnostics
      )
      
      writer.foreach { w =>
        w.print(JsonMessage.serialize(response))
        w.flush()
      }: Option
    }
  }

  def handleDocumentCheck(msg: JsonMessage, writer: Option[PrintWriter]): Unit = {
    val payload = PayloadParser.extractDocumentCheck(msg.payload)
    
    payload.foreach { p =>
      println(s"Document check: ${p.uri} version ${p.version}")
      
      val diagnostics = generateMockDiagnostics(p.uri)
      
      val response = JsonMessage.createDiagnostics(
        session = msg.session.getOrElse("default"),
        version = msg.version.getOrElse(1L),
        diagnostics = diagnostics
      )
      
      writer.foreach { w =>
        w.print(JsonMessage.serialize(response))
        w.flush()
      }: Option
    }
  }

  def handleMarkup(msg: JsonMessage, writer: Option[PrintWriter]): Unit = {
    val payload = PayloadParser.extractMarkupPayload(msg.payload)
    
    payload.foreach { p =>
      println(s"Markup request: ${p.uri} at line ${p.offset.line}, col ${p.offset.col}")
      
      val info = if (mockMode) {
        s"theorem foo: some theorem at line ${p.offset.line}"
      } else {
        generateIsabelleMarkup(p.uri, p.offset.line, p.offset.col)
      }
      
      val response = JsonMessage.createMarkup(
        uri = p.uri,
        offset = p.offset,
        info = info,
        session = msg.session.getOrElse("default"),
        version = msg.version.getOrElse(1L)
      )
      
      writer.foreach { w =>
        w.print(JsonMessage.serialize(response))
        w.flush()
      }: Option
    }
  }

  def handleMessageMock(msg: JsonMessage): Unit = {
    val msgType = MessageType.fromString(msg.`type`)
    
    msgType match {
      case DocumentPush =>
        val payload = PayloadParser.extractDocumentPush(msg.payload)
        payload.foreach { p =>
          println(s"[MOCK] Document push: ${p.uri}")
          
          val response = JsonMessage.createDiagnostics(
            session = msg.session.getOrElse("s1"),
            version = msg.version.getOrElse(1L),
            diagnostics = generateMockDiagnostics(p.uri)
          )
          
          print(JsonMessage.serialize(response))
          scala.Predef.flush()
        }
        
      case DocumentCheck =>
        val payload = PayloadParser.extractDocumentCheck(msg.payload)
        payload.foreach { p =>
          println(s"[MOCK] Document check: ${p.uri}")
          
          val response = JsonMessage.createDiagnostics(
            session = msg.session.getOrElse("s1"),
            version = msg.version.getOrElse(1L),
            diagnostics = generateMockDiagnostics(p.uri)
          )
          
          print(JsonMessage.serialize(response))
          scala.Predef.flush()
        }
        
      case Markup =>
        val payload = PayloadParser.extractMarkupPayload(msg.payload)
        payload.foreach { p =>
          println(s"[MOCK] Markup: ${p.uri}")
          
          val response = JsonMessage.createMarkup(
            uri = p.uri,
            offset = p.offset,
            info = s"Mock markup info at line ${p.offset.line}",
            session = msg.session.getOrElse("s1"),
            version = msg.version.getOrElse(1L)
          )
          
          print(JsonMessage.serialize(response))
          scala.Predef.flush()
        }
        
      case _ =>
        println(s"[MOCK] Unknown message type: ${msg.`type`}")
    }
  }

  def generateMockDiagnostics(uri: String): List[Diagnostic] = {
    List(
      Diagnostic(
        uri = uri,
        range = Range(Position(1, 0), Position(1, 6)),
        severity = "error",
        message = "Parse error (mock)"
      )
    )
  }

  def generateIsabelleDiagnostics(uri: String, text: String): List[Diagnostic] = {
    List(
      Diagnostic(
        uri = uri,
        range = Range(Position(0, 0), Position(0, 5)),
        severity = "info",
        message = "Isabelle session started (placeholder)"
      )
    )
  }

  def generateIsabelleMarkup(uri: String, line: Long, col: Long): String = {
    s"term at $line:$col"
  }
}
