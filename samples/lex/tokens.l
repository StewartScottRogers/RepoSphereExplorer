/* A scanner for the expression language the Yacc fixture parses. Written
   to exercise every field the plugin extracts: options, an exclusive
   start condition, named definitions, rules that use them, a conditional
   rule, a catch-all, and a definition nothing uses. */

%{
#include <stdlib.h>
#include "expression.tab.h"
%}

%option noyywrap
%option yylineno
%option nounput

%x COMMENT

DIGIT       [0-9]
LETTER      [a-zA-Z_]
WHITESPACE  [ \t\r]
RETIRED     [$@]

%%

{DIGIT}+"."{DIGIT}+     { yylval.number = atof(yytext); return NUMBER; }
{DIGIT}+                { yylval.number = atof(yytext); return NUMBER; }
{LETTER}({LETTER}|{DIGIT})*  { yylval.name = strdup(yytext); return IDENTIFIER; }

"+"                     { return PLUS; }
"-"                     { return MINUS; }
"*"                     { return TIMES; }
"/"                     { return DIVIDE; }
"("                     { return LPAREN; }
")"                     { return RPAREN; }
"="                     { return ASSIGN; }

"/*"                    { BEGIN(COMMENT); }
<COMMENT>"*/"           { BEGIN(INITIAL); }
<COMMENT>\n             ;
<COMMENT>.              ;

{WHITESPACE}+           ;
\n                      { return NEWLINE; }

.                       { return yytext[0]; }

%%

int yylex_destroy_all(void) {
  return yylex_destroy();
}
