;; Base highlighting borrowed from tree-sitter-sml and extended with common
;; Isabelle tokens that the SML grammar treats as identifiers.

;; Comments
[(block_comment) (line_comment)] @comment

;; SML reserved words
[
 "abstype" "and" "andalso" "as" "case" "datatype" "do" "else" "end"
 "exception" "fn" "fun" "handle" "if" "in" "infix" "infixr" "let"
 "local" "nonfix" "of" "op" "open" "orelse" "raise" "rec" "then"
 "type" "val" "with" "withtype" "while"
 "eqtype" "functor" "include" "sharing" "sig" "signature" "struct"
 "structure" "where"
] @keyword

;; Isabelle proof/theory words (parsed as identifiers by tree-sitter-sml)
((vid) @keyword
 (#match? @keyword "^(theory|imports|begin|end|locale|context|lemma|theorem|corollary|proposition|definition|abbreviation|notation|axiomatization|inductive|coinductive|primrec|fun|termination|proof|qed|done|apply|using|unfolding|by|have|show|thus|hence|from|assumes|shows|fixes|obtains|defines|where|let|in|if|then|else|for)$"))

;; Literals
[(integer_scon) (word_scon) (real_scon)] @number
[(string_scon) (char_scon)] @string

;; Types
(fn_ty "->" @type)
(tuple_ty "*" @type)
(paren_ty ["(" ")"] @type)
(tyvar_ty (tyvar) @type)
(record_ty
 ["{" "," "}"] @type
 (tyrow [(lab) ":"] @type)?
 (ellipsis_tyrow ["..." ":"] @type)?)
(tycon_ty
 (tyseq ["(" "," ")"] @type)?
 (longtycon) @type)

;; Constructors
((vid) @constructor
 (#match? @constructor "^[A-Z].*"))
(longvid ((vid) @vid
          (#match? @vid "^[A-Z].*"))) @constructor

;; Built-in constructors
((vid) @constant.builtin
 (#match? @constant.builtin "^(true|false|nil|::|ref)$"))
(longvid ((vid) @vid
          (#match? @vid "^(true|false|nil|::|ref)$"))) @constant.builtin

;; Punctuation
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ":" ";" "|" "=>" ":>"] @punctuation.delimiter
