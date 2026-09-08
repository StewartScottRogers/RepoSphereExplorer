% A route planner over a small rail network: facts for the lines, rules
% for reachability and cost, and a shortest-path search that will not loop
% on a cycle. Runs in SWI-Prolog.

:- module(planner, [
       route/3,
       route/4,
       shortest_route/3,
       reachable/2,
       station/1,
       interchange/1,
       longest_hop/1
   ]).

:- use_module(library(lists)).

% -- facts ---------------------------------------------------------------

station(kings_cross).
station(farringdon).
station(moorgate).
station(bank).
station(waterloo).
station(paddington).
station(baker_street).

% link(From, To, Minutes, Line).
link(kings_cross, farringdon, 4, metropolitan).
link(farringdon, moorgate, 3, metropolitan).
link(moorgate, bank, 2, northern).
link(bank, waterloo, 5, waterloo_city).
link(kings_cross, baker_street, 6, circle).
link(baker_street, paddington, 5, bakerloo).
link(paddington, farringdon, 9, circle).
link(waterloo, paddington, 12, bakerloo).

% -- rules ---------------------------------------------------------------

%! connected(?From, ?To, ?Minutes, ?Line) is nondet.
%  Links are usable in both directions.
connected(From, To, Minutes, Line) :- link(From, To, Minutes, Line).
connected(From, To, Minutes, Line) :- link(To, From, Minutes, Line).

%! interchange(?Station) is nondet.
%  A station served by more than one line.
interchange(Station) :-
    station(Station),
    setof(Line, To^Minutes^connected(Station, To, Minutes, Line), Lines),
    length(Lines, Count),
    Count > 1.

%! route(+From, +To, -Path) is nondet.
route(From, To, Path) :-
    route(From, To, Path, _Minutes).

%! route(+From, +To, -Path, -Minutes) is nondet.
route(From, To, Path, Minutes) :-
    walk(From, To, [From], Reversed, 0, Minutes),
    reverse(Reversed, Path).

walk(Station, Station, Visited, Visited, Minutes, Minutes).
walk(From, To, Visited, Path, SoFar, Minutes) :-
    connected(From, Next, Cost, _Line),
    \+ memberchk(Next, Visited),
    Running is SoFar + Cost,
    walk(Next, To, [Next | Visited], Path, Running, Minutes).

%! shortest_route(+From, +To, -Path) is semidet.
shortest_route(From, To, Path) :-
    findall(Minutes-Candidate, route(From, To, Candidate, Minutes), Candidates),
    Candidates \= [],
    keysort(Candidates, [_Best-Path | _Rest]).

%! reachable(+From, -Station) is nondet.
reachable(From, Station) :-
    route(From, Station, _Path),
    Station \= From.

%! longest_hop(-Link) is semidet.
longest_hop(link(From, To, Minutes, Line)) :-
    findall(M-link(F, T, M, L), link(F, T, M, L), Hops),
    keysort(Hops, Sorted),
    last(Sorted, Minutes-link(From, To, Minutes, Line)).

%! line_of(?Line, -Stations) is nondet.
line_of(Line, Stations) :-
    setof(Station, To^Minutes^connected(Station, To, Minutes, Line), Stations).

% -- entry point ---------------------------------------------------------

report :-
    shortest_route(kings_cross, waterloo, Path),
    format("shortest: ~w~n", [Path]),
    forall(interchange(Station), format("interchange: ~w~n", [Station])),
    longest_hop(Hop),
    format("longest hop: ~w~n", [Hop]).

:- initialization(report, main).
