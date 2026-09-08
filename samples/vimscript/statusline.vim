" statusline.vim - a self-contained statusline with git, diagnostics and
" a mode indicator, plus the autocommands and mappings that drive it.
"
" Drop into ~/.vim/plugin/ or source it directly.

if exists('g:loaded_rse_statusline')
  finish
endif
let g:loaded_rse_statusline = 1

let g:rse_statusline_show_git = get(g:, 'rse_statusline_show_git', 1)
let g:rse_statusline_max_path = get(g:, 'rse_statusline_max_path', 40)
let g:rse_statusline_symbols = get(g:, 'rse_statusline_symbols', {
      \ 'modified': '+',
      \ 'readonly': 'RO',
      \ 'branch':   'git:',
      \ 'error':    'E',
      \ 'warning':  'W',
      \ })

let s:mode_names = {
      \ 'n': 'NORMAL', 'i': 'INSERT', 'v': 'VISUAL', 'V': 'V-LINE',
      \ "\<C-v>": 'V-BLOCK', 'c': 'COMMAND', 'R': 'REPLACE', 't': 'TERMINAL',
      \ }

let s:branch_cache = {}

function! s:Shorten(path, limit) abort
  if strlen(a:path) <= a:limit
    return a:path
  endif
  return '...' . strpart(a:path, strlen(a:path) - a:limit + 3)
endfunction

function! RseModeName() abort
  return get(s:mode_names, mode(), mode())
endfunction

function! RseGitBranch(...) abort
  if !g:rse_statusline_show_git
    return ''
  endif

  let l:root = a:0 > 0 ? a:1 : expand('%:p:h')
  if has_key(s:branch_cache, l:root)
    return s:branch_cache[l:root]
  endif

  let l:head = findfile('.git/HEAD', l:root . ';')
  if empty(l:head)
    let s:branch_cache[l:root] = ''
    return ''
  endif

  let l:lines = readfile(l:head, '', 1)
  let l:branch = empty(l:lines) ? '' : substitute(l:lines[0], '^ref: refs/heads/', '', '')
  let s:branch_cache[l:root] = l:branch
  return l:branch
endfunction

function! RseFileFlags() abort
  let l:flags = []
  if &modified
    call add(l:flags, g:rse_statusline_symbols.modified)
  endif
  if &readonly || !&modifiable
    call add(l:flags, g:rse_statusline_symbols.readonly)
  endif
  return empty(l:flags) ? '' : '[' . join(l:flags, ' ') . ']'
endfunction

function! RseDiagnostics() abort
  if !exists('*getloclist')
    return ''
  endif

  let l:items = getloclist(0)
  if empty(l:items)
    return ''
  endif

  let l:errors = len(filter(copy(l:items), 'get(v:val, "type", "") ==# "E"'))
  let l:warnings = len(l:items) - l:errors
  let l:parts = []
  if l:errors > 0
    call add(l:parts, g:rse_statusline_symbols.error . l:errors)
  endif
  if l:warnings > 0
    call add(l:parts, g:rse_statusline_symbols.warning . l:warnings)
  endif
  return join(l:parts, ' ')
endfunction

function! RseStatusline(active) abort
  let l:path = s:Shorten(expand('%:~:.'), g:rse_statusline_max_path)
  if empty(l:path)
    let l:path = '[No Name]'
  endif

  if !a:active
    return ' ' . l:path . ' '
  endif

  let l:branch = RseGitBranch()
  let l:segments = [' ' . RseModeName(), l:path, RseFileFlags()]
  if !empty(l:branch)
    call add(l:segments, g:rse_statusline_symbols.branch . l:branch)
  endif

  let l:right = [RseDiagnostics(), &filetype, '%l:%c', '%p%%']
  return join(filter(l:segments, '!empty(v:val)'), ' ')
        \ . '%=' . join(filter(l:right, '!empty(v:val)'), ' ') . ' '
endfunction

function! s:Refresh() abort
  for l:window in range(1, winnr('$'))
    call setwinvar(l:window, '&statusline', '%!RseStatusline(' . (l:window == winnr()) . ')')
  endfor
endfunction

function! RseClearBranchCache() abort
  let s:branch_cache = {}
  call s:Refresh()
endfunction

command! -bar RseStatuslineRefresh call <SID>Refresh()
command! -bar RseStatuslineClearCache call RseClearBranchCache()

augroup rse_statusline
  autocmd!
  autocmd VimEnter,WinEnter,BufWinEnter * call <SID>Refresh()
  autocmd BufWritePost * call RseClearBranchCache()
augroup END

nnoremap <silent> <Leader>sr :RseStatuslineRefresh<CR>
nnoremap <silent> <Leader>sc :RseStatuslineClearCache<CR>

call s:Refresh()
