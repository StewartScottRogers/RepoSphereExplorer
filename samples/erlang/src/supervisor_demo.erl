%%%-------------------------------------------------------------------
%%% @doc A worker pool with a supervisor-shaped restart policy, written
%%% as plain OTP-style Erlang: a gen_server holding state, a public API
%%% in front of it, and the callbacks underneath.
%%% @end
%%%-------------------------------------------------------------------
-module(supervisor_demo).

-behaviour(gen_server).

%% Public API
-export([start_link/1, submit/2, stats/1, drain/1, stop/1]).

%% gen_server callbacks
-export([init/1, handle_call/3, handle_cast/2, handle_info/2, terminate/2,
         code_change/3]).

-define(SERVER, ?MODULE).
-define(DEFAULT_RESTART_LIMIT, 3).

-record(worker, {ref :: reference(),
                 pid :: pid(),
                 restarts = 0 :: non_neg_integer()}).

-record(state, {workers = [] :: [#worker{}],
                queue = [] :: [term()],
                done = 0 :: non_neg_integer(),
                failed = 0 :: non_neg_integer(),
                restart_limit = ?DEFAULT_RESTART_LIMIT :: pos_integer()}).

-type stats() :: #{done := non_neg_integer(),
                   failed := non_neg_integer(),
                   queued := non_neg_integer(),
                   workers := non_neg_integer()}.

-export_type([stats/0]).

%%====================================================================
%% API
%%====================================================================

-spec start_link(pos_integer()) -> {ok, pid()} | {error, term()}.
start_link(PoolSize) when is_integer(PoolSize), PoolSize > 0 ->
    gen_server:start_link({local, ?SERVER}, ?MODULE, [PoolSize], []).

-spec submit(pid(), term()) -> ok.
submit(Server, Job) ->
    gen_server:cast(Server, {submit, Job}).

-spec stats(pid()) -> stats().
stats(Server) ->
    gen_server:call(Server, stats).

-spec drain(pid()) -> {ok, non_neg_integer()}.
drain(Server) ->
    gen_server:call(Server, drain, 30000).

-spec stop(pid()) -> ok.
stop(Server) ->
    gen_server:stop(Server).

%%====================================================================
%% gen_server callbacks
%%====================================================================

init([PoolSize]) ->
    process_flag(trap_exit, true),
    Workers = [spawn_worker() || _ <- lists:seq(1, PoolSize)],
    {ok, #state{workers = Workers}}.

handle_call(stats, _From, State) ->
    Reply = #{done => State#state.done,
              failed => State#state.failed,
              queued => length(State#state.queue),
              workers => length(State#state.workers)},
    {reply, Reply, State};
handle_call(drain, _From, State = #state{queue = Queue}) ->
    Drained = length(Queue),
    {reply, {ok, Drained}, State#state{queue = []}};
handle_call(Request, _From, State) ->
    {reply, {error, {unknown_request, Request}}, State}.

handle_cast({submit, Job}, State = #state{queue = Queue}) ->
    {noreply, dispatch(State#state{queue = Queue ++ [Job]})};
handle_cast(_Msg, State) ->
    {noreply, State}.

handle_info({'EXIT', Pid, normal}, State) ->
    {noreply, State#state{done = State#state.done + 1,
                          workers = replace(Pid, State#state.workers)}};
handle_info({'EXIT', Pid, Reason}, State) ->
    error_logger:warning_msg("worker ~p died: ~p~n", [Pid, Reason]),
    {noreply, State#state{failed = State#state.failed + 1,
                          workers = replace(Pid, State#state.workers)}};
handle_info(_Info, State) ->
    {noreply, State}.

terminate(_Reason, #state{workers = Workers}) ->
    [exit(Worker#worker.pid, shutdown) || Worker <- Workers],
    ok.

code_change(_OldVsn, State, _Extra) ->
    {ok, State}.

%%====================================================================
%% Internals
%%====================================================================

spawn_worker() ->
    Pid = spawn_link(fun worker_loop/0),
    #worker{ref = make_ref(), pid = Pid}.

worker_loop() ->
    receive
        {job, Payload} ->
            handle_job(Payload),
            worker_loop();
        stop ->
            ok
    after 60000 ->
        ok
    end.

handle_job({fail, Reason}) ->
    exit(Reason);
handle_job(Payload) ->
    io:format("handled ~p~n", [Payload]).

dispatch(State = #state{queue = []}) ->
    State;
dispatch(State = #state{queue = [Job | Rest], workers = [Worker | Others]}) ->
    Worker#worker.pid ! {job, Job},
    dispatch(State#state{queue = Rest, workers = Others ++ [Worker]});
dispatch(State) ->
    State.

replace(Pid, Workers) ->
    Remaining = [W || W <- Workers, W#worker.pid =/= Pid],
    [spawn_worker() | Remaining].
