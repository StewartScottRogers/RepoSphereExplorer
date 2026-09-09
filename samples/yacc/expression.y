/* A calculator grammar. Written to exercise every field the plugin
   extracts: a union, typed tokens, precedence in both directions, a start
   symbol, rules with empty alternatives and actions, and an error rule. */

%{
#include <stdio.h>
#include <stdlib.h>

int yylex(void);
void yyerror(const char *message);
%}

%union {
  double number;
  char *name;
}

%token <number> NUMBER
%token <name> IDENTIFIER
%token PLUS MINUS TIMES DIVIDE
%token LPAREN RPAREN ASSIGN NEWLINE

%left PLUS MINUS
%left TIMES DIVIDE
%right UMINUS

%type <number> expression term

%start program

%%

program
  : statements
  ;

statements
  : statements statement
  |
  ;

statement
  : expression NEWLINE          { printf("= %g\n", $1); }
  | IDENTIFIER ASSIGN expression NEWLINE
  | error NEWLINE               { yyerrok; }
  ;

expression
  : expression PLUS expression  { $$ = $1 + $3; }
  | expression MINUS expression { $$ = $1 - $3; }
  | expression TIMES expression { $$ = $1 * $3; }
  | expression DIVIDE expression
  | MINUS expression %prec UMINUS
  | term
  ;

term
  : NUMBER                      { $$ = $1; }
  | LPAREN expression RPAREN    { $$ = $2; }
  ;

%%

void yyerror(const char *message) {
  fprintf(stderr, "%s\n", message);
}

int main(void) {
  return yyparse();
}
