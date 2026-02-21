package isabelle.adapter

import org.json4s._
import org.json4s.native.Serialization
import org.json4s.native.Serialization.{write, read}
import org.scalatest.flatspec.AnyFlatSpec
import org.scalatest.matchers.should.Matchers
import org.scalatestplus.mockito.MockitoSugar
import org.mockito.Mockito._

class AdapterSpec extends AnyFlatSpec with Matchers with MockitoSugar {

  implicit val formats = Serialization.formats(NoTypeHints)

  "JsonMessage" should "parse document.push message" in {
    val json = """{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}"""
    
    val msg = JsonMessage.parse(json)
    
    msg should not be None
    msg.get.id should equal("msg-0001")
    msg.get.`type` should equal("document.push")
    msg.get.session should equal(Some("s1"))
    msg.get.version should equal(Some(1))
  }

  it should "parse diagnostics message" in {
    val json = """{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":{"diagnostics":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}}"""
    
    val msg = JsonMessage.parse(json)
    
    msg should not be None
    msg.get.`type` should equal("diagnostics")
  }

  it should "parse markup message" in {
    val json = """{"id":"msg-0002","type":"markup","session":"s1","version":1,"payload":{"uri":"file:///test.thy","offset":{"line":5,"col":10},"info":"theorem foo: ..."}}"""
    
    val msg = JsonMessage.parse(json)
    
    msg should not be None
    msg.get.`type` should equal("markup")
  }

  "PayloadParser" should "extract DocumentPushPayload" in {
    val payload = JObject(
      "uri" -> JString("file:///test.thy"),
      "text" -> JString("theory Test begin end")
    )
    
    val result = PayloadParser.extractDocumentPush(payload)
    
    result should not be None
    result.get.uri should equal("file:///test.thy")
    result.get.text should equal("theory Test begin end")
  }

  it should "extract DocumentCheckPayload" in {
    val payload = JObject(
      "uri" -> JString("file:///test.thy"),
      "version" -> JInt(5)
    )
    
    val result = PayloadParser.extractDocumentCheck(payload)
    
    result should not be None
    result.get.uri should equal("file:///test.thy")
    result.get.version should equal(5)
  }

  it should "extract MarkupPayload" in {
    val payload = JObject(
      "uri" -> JString("file:///test.thy"),
      "offset" -> JObject("line" -> JInt(10), "col" -> JInt(5)),
      "info" -> JString("theorem foo")
    )
    
    val result = PayloadParser.extractMarkupPayload(payload)
    
    result should not be None
    result.get.uri should equal("file:///test.thy")
    result.get.offset.line should equal(10)
    result.get.offset.col should equal(5)
  }

  "MessageType" should "convert from string" in {
    MessageType.fromString("document.push") should equal(DocumentPush)
    MessageType.fromString("document.check") should equal(DocumentCheck)
    MessageType.fromString("diagnostics") should equal(Diagnostics)
    MessageType.fromString("markup") should equal(Markup)
    MessageType.fromString("unknown") should equal(Unknown)
  }

  it should "convert to string" in {
    MessageType.toString(DocumentPush) should equal("document.push")
    MessageType.toString(DocumentCheck) should equal("document.check")
    MessageType.toString(Diagnostics) should equal("diagnostics")
    MessageType.toString(Markup) should equal("markup")
  }

  "JsonMessage.createDocumentPush" should "create valid message" in {
    val msg = JsonMessage.createDocumentPush("file:///test.thy", "theory Test", "session1", 5)
    
    msg.id should not be empty
    msg.`type` should equal("document.push")
    msg.session should equal(Some("session1"))
    msg.version should equal(Some(5))
  }

  "JsonMessage.createDiagnostics" should "create valid diagnostics message" in {
    val diagnostics = List(
      Diagnostic(
        uri = "file:///test.thy",
        range = Range(Position(1, 0), Position(1, 6)),
        severity = "error",
        message = "Parse error"
      )
    )
    
    val msg = JsonMessage.createDiagnostics("s1", 1, diagnostics)
    
    msg.id should not be empty
    msg.`type` should equal("diagnostics")
    msg.session should equal(Some("s1"))
    msg.version should equal(Some(1))
  }

  "JsonMessage.serialize" should "serialize and deserialize roundtrip" in {
    val original = JsonMessage.createDocumentPush("file:///test.thy", "theory Test", "session1", 5)
    val serialized = JsonMessage.serialize(original)
    
    serialized should include("document.push")
    serialized should include("file:///test.thy")
    serialized should include("\n")
  }
}
