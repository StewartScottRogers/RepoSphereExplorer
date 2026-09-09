// A combined grammar for a small expression language. Written to exercise
// every field the plugin extracts: options, declared tokens, channels,
// parser rules with alternatives spanning several lines, lexer rules and
// a fragment.

grammar Expr;

// Rules shared with the other grammars in this directory, brought in
// rather than repeated.
import CommonLexerRules;

options { language = Java; }

tokens { PLUS, MINUS, TIMES, DIVIDE }

channels { COMMENTS, WHITESPACE }

// Parser rules: lower case.

prog
  : statement+ EOF
  ;

statement
  : expression ';'
  | ID '=' expression ';'
  | 'print' expression ';'
  ;

expression
  : expression op=('*' | '/') expression
  | expression op=('+' | '-') expression
  | '(' expression ')'
  | atom
  ;

atom
  : INT
  | FLOAT
  | ID
  | STRING
  ;

// Lexer rules: initial capital.

ID: LETTER (LETTER | DIGIT | '_')*;

INT: DIGIT+;

FLOAT: DIGIT+ '.' DIGIT+;

STRING: '"' (~["\\] | '\\' .)* '"';

LINE_COMMENT: '//' ~[\r\n]* -> channel(COMMENTS);

WS: [ \t\r\n]+ -> skip;

// Fragments: usable only from another lexer rule, never on their own.

fragment LETTER: [a-zA-Z];

fragment DIGIT: [0-9];
