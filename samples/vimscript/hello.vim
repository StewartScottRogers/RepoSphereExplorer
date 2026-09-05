set nocompatible
syntax on

function! Greet(name)
    echo 'Hello, ' . a:name
endfunction

let g:greeting_enabled = 1
