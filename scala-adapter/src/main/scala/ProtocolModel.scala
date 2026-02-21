package isabelle.adapter

import io.circe.Decoder
import io.circe.Encoder
import io.circe.Json
import io.circe.parser
import io.circe.syntax.*
import io.circe.generic.semiauto.*

object ProtocolModel {
  val DocumentPushExample: String =
    """{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\\nend\\n"}}"""

  val DiagnosticsExample: String =
    """{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}"""

  sealed trait MessageType {
    def value: String
  }

  object MessageType {
    case object DocumentPush extends MessageType {
      val value: String = "document.push"
    }
    case object DocumentCheck extends MessageType {
      val value: String = "document.check"
    }
    case object Diagnostics extends MessageType {
      val value: String = "diagnostics"
    }
    case object Markup extends MessageType {
      val value: String = "markup"
    }

    def fromString(value: String): Either[String, MessageType] = value match {
      case "document.push"  => Right(DocumentPush)
      case "document.check" => Right(DocumentCheck)
      case "diagnostics"    => Right(Diagnostics)
      case "markup"         => Right(Markup)
      case other             => Left(s"Unsupported message type: $other")
    }
  }

  final case class Position(line: Int, col: Int)
  final case class Range(start: Position, end: Position)

  final case class Diagnostic(
    uri: String,
    range: Range,
    severity: String,
    message: String
  )

  final case class DocumentPushPayload(uri: String, text: String)
  final case class DocumentCheckPayload(uri: String, version: Int)
  final case class MarkupPayload(uri: String, offset: Position, info: String)

  final case class Envelope(
    id: String,
    `type`: String,
    session: String,
    version: Int,
    payload: Json
  )

  sealed trait IncomingMessage {
    def envelope: Envelope
  }

  final case class DocumentPushRequest(envelope: Envelope, payload: DocumentPushPayload)
      extends IncomingMessage
  final case class DocumentCheckRequest(envelope: Envelope, payload: DocumentCheckPayload)
      extends IncomingMessage
  final case class MarkupRequest(envelope: Envelope, payload: MarkupPayload)
      extends IncomingMessage

  given Encoder[Position] = deriveEncoder
  given Decoder[Position] = deriveDecoder
  given Encoder[Range] = deriveEncoder
  given Decoder[Range] = deriveDecoder
  given Encoder[Diagnostic] = deriveEncoder
  given Decoder[Diagnostic] = deriveDecoder
  given Encoder[DocumentPushPayload] = deriveEncoder
  given Decoder[DocumentPushPayload] = deriveDecoder
  given Encoder[DocumentCheckPayload] = deriveEncoder
  given Decoder[DocumentCheckPayload] = deriveDecoder
  given Encoder[MarkupPayload] = deriveEncoder
  given Decoder[MarkupPayload] = deriveDecoder
  given Encoder[Envelope] = deriveEncoder
  given Decoder[Envelope] = deriveDecoder

  def decodeEnvelope(line: String): Either[String, Envelope] =
    parser.decode[Envelope](line).left.map(_.getMessage)

  def decodeIncoming(envelope: Envelope): Either[String, IncomingMessage] =
    MessageType.fromString(envelope.`type`).flatMap {
      case MessageType.DocumentPush =>
        envelope.payload
          .as[DocumentPushPayload]
          .left
          .map(_.getMessage)
          .map(DocumentPushRequest(envelope, _))
      case MessageType.DocumentCheck =>
        envelope.payload
          .as[DocumentCheckPayload]
          .left
          .map(_.getMessage)
          .map(DocumentCheckRequest(envelope, _))
      case MessageType.Markup =>
        envelope.payload
          .as[MarkupPayload]
          .left
          .map(_.getMessage)
          .map(MarkupRequest(envelope, _))
      case MessageType.Diagnostics =>
        Left("Incoming diagnostics messages are unsupported")
    }

  def encodeEnvelope(envelope: Envelope): String =
    envelope.asJson.noSpaces

  def diagnosticsResponse(envelope: Envelope, diagnostics: List[Diagnostic]): Envelope =
    Envelope(
      id = envelope.id,
      `type` = MessageType.Diagnostics.value,
      session = envelope.session,
      version = envelope.version,
      payload = diagnostics.asJson
    )

  def markupResponse(envelope: Envelope, uri: String, offset: Position, info: String): Envelope =
    Envelope(
      id = envelope.id,
      `type` = MessageType.Markup.value,
      session = envelope.session,
      version = envelope.version,
      payload = MarkupPayload(uri = uri, offset = offset, info = info).asJson
    )
}
