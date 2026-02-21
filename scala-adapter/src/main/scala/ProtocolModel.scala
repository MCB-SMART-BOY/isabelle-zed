package isabelle.adapter

import org.json4s._
import org.json4s.native.Serialization
import org.json4s.native.Serialization.{write, read}
import org.json4s.DefaultFormats

sealed trait MessageType
case object DocumentPush extends MessageType
case object DocumentCheck extends MessageType
case object Diagnostics extends MessageType
case object Markup extends MessageType
case object Unknown extends MessageType

object MessageType {
  def fromString(s: String): MessageType = s match {
    case "document.push" => DocumentPush
    case "document.check" => DocumentCheck
    case "diagnostics" => Diagnostics
    case "markup" => Markup
    case _ => Unknown
  }

  def toString(mt: MessageType): String = mt match {
    case DocumentPush => "document.push"
    case DocumentCheck => "document.check"
    case Diagnostics => "diagnostics"
    case Markup => "markup"
    case Unknown => "unknown"
  }
}

case class Position(line: Long, col: Long)

case class Range(start: Position, end: Position)

sealed trait DiagnosticSeverity
case object Error extends DiagnosticSeverity
case object Warning extends DiagnosticSeverity
case object Info extends DiagnosticSeverity

object DiagnosticSeverity {
  def fromString(s: String): DiagnosticSeverity = s.toLowerCase match {
    case "error" => Error
    case "warning" => Warning
    case "info" => Info
    case _ => Error
  }

  def toString(ds: DiagnosticSeverity): String = ds match {
    case Error => "error"
    case Warning => "warning"
    case Info => "info"
  }
}

case class Diagnostic(
  uri: String,
  range: Range,
  severity: String,
  message: String
)

case class DocumentPushPayload(uri: String, text: String)

case class DocumentCheckPayload(uri: String, version: Long)

case class DiagnosticsPayload(diagnostics: List[Diagnostic])

case class MarkupPayload(uri: String, offset: Position, info: String)

case class JsonMessage(
  id: String,
  `type`: String,
  session: Option[String] = None,
  version: Option[Long] = None,
  payload: JValue = JObject()
)

object JsonMessage {
  implicit val formats: Formats = DefaultFormats

  def parse(line: String): Option[JsonMessage] = {
    try {
      Some(read[JsonMessage](line))
    } catch {
      case e: Exception =>
        System.err.println(s"Parse error: ${e.getMessage}")
        None
    }
  }

  def serialize(msg: JsonMessage): String = {
    write(msg) + "\n"
  }

  def createDocumentPush(uri: String, text: String, session: String, version: Long): JsonMessage = {
    JsonMessage(
      id = s"msg-${System.currentTimeMillis() % 10000}",
      `type` = "document.push",
      session = Some(session),
      version = Some(version),
      payload = JObject(
        "uri" -> JString(uri),
        "text" -> JString(text)
      )
    )
  }

  def createDiagnostics(
    session: String,
    version: Long,
    diagnostics: List[Diagnostic]
  ): JsonMessage = {
    JsonMessage(
      id = s"msg-${System.currentTimeMillis() % 10000}",
      `type` = "diagnostics",
      session = Some(session),
      version = Some(version),
      payload = JObject(
        "diagnostics" -> JArray(diagnostics.map { d =>
          JObject(
            "uri" -> JString(d.uri),
            "range" -> JObject(
              "start" -> JObject("line" -> JInt(d.range.start.line), "col" -> JInt(d.range.start.col)),
              "end" -> JObject("line" -> JInt(d.range.end.line), "col" -> JInt(d.range.end.col))
            ),
            "severity" -> JString(d.severity),
            "message" -> JString(d.message)
          )
        })
      )
    )
  }

  def createMarkup(
    uri: String,
    offset: Position,
    info: String,
    session: String,
    version: Long
  ): JsonMessage = {
    JsonMessage(
      id = s"msg-${System.currentTimeMillis() % 10000}",
      `type` = "markup",
      session = Some(session),
      version = Some(version),
      payload = JObject(
        "uri" -> JString(uri),
        "offset" -> JObject("line" -> JInt(offset.line), "col" -> JInt(offset.col)),
        "info" -> JString(info)
      )
    )
  }
}

object PayloadParser {
  def extractDocumentPush(payload: JValue): Option[DocumentPushPayload] = {
    try {
      Some(DocumentPushPayload(
        uri = (payload \ "uri").extract[String],
        text = (payload \ "text").extract[String]
      ))
    } catch {
      case _: Exception => None
    }
  }

  def extractDocumentCheck(payload: JValue): Option[DocumentCheckPayload] = {
    try {
      Some(DocumentCheckPayload(
        uri = (payload \ "uri").extract[String],
        version = (payload \ "version").extract[Long]
      ))
    } catch {
      case _: Exception => None
    }
  }

  def extractMarkupPayload(payload: JValue): Option[MarkupPayload] = {
    try {
      Some(MarkupPayload(
        uri = (payload \ "uri").extract[String],
        offset = Position(
          line = (payload \ "offset" \ "line").extract[Long],
          col = (payload \ "offset" \ "col").extract[Long]
        ),
        info = (payload \ "info").extract[String]
      ))
    } catch {
      case _: Exception => None
    }
  }
}
