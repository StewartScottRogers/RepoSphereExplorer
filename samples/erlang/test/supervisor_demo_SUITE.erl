-module(supervisor_demo_SUITE).

-export([all/0, init_per_testcase/2, end_per_testcase/2]).
-export([submits_work/1, reports_statistics/1, drains_before_stopping/1]).

-include_lib("common_test/include/ct.hrl").

all() ->
    [submits_work, reports_statistics, drains_before_stopping].

init_per_testcase(_Case, Config) ->
    {ok, Pid} = supervisor_demo:start_link(2),
    [{server, Pid} | Config].

end_per_testcase(_Case, Config) ->
    %% Stopping an already stopped server would fail the case for a reason
    %% that has nothing to do with what it was testing.
    Pid = ?config(server, Config),
    case is_process_alive(Pid) of
        true -> supervisor_demo:stop(Pid);
        false -> ok
    end.

submits_work(Config) ->
    Pid = ?config(server, Config),
    ok = supervisor_demo:submit(Pid, fun() -> 21 * 2 end),
    ok.

reports_statistics(Config) ->
    Pid = ?config(server, Config),
    ok = supervisor_demo:submit(Pid, fun() -> ok end),
    Stats = supervisor_demo:stats(Pid),
    true = is_map(Stats) orelse is_tuple(Stats),
    ok.

drains_before_stopping(Config) ->
    Pid = ?config(server, Config),
    ok = supervisor_demo:submit(Pid, fun() -> ok end),
    _ = supervisor_demo:drain(Pid),
    ok.
