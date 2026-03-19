;; Highlight rules aligned with tree-sitter-isabelle grammar nodes.

(comment) @comment
(inner_syntax) @string

;; Core theory/proof keywords
[
  "theory"
  "imports"
  "begin"
  "end"
  "lemma"
  "theorem"
  "definition"
  "datatype"
  "proof"
  "qed"
  "apply"
  "done"
  "by"
  "assume"
  "have"
  "show"
  "where"
  "sorry"
] @keyword

;; Command/term names
(theory_definition (identifier) @type)
(lemma_command (identifier) @function)
(theorem_command (identifier) @function)
(definition_command (identifier) @function)
(datatype_command (identifier) @type)
(method (identifier) @function.method)
(identifier) @variable

;; Punctuation/operators
["(" ")"] @punctuation.bracket
[":" "::" "=" "|"] @operator
