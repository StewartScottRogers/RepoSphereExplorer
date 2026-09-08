(* A least-recently-used cache built from a hash table and a doubly linked
   list, with a functor so the key type is a parameter, and a module type
   stating what a cache offers. *)

module type KEY = sig
  type t

  val equal : t -> t -> bool
  val hash : t -> int
  val to_string : t -> string
end

module type CACHE = sig
  type key
  type 'a t

  val create : int -> 'a t
  val capacity : 'a t -> int
  val size : 'a t -> int
  val put : 'a t -> key -> 'a -> unit
  val find_opt : 'a t -> key -> 'a option
  val remove : 'a t -> key -> bool
  val keys : 'a t -> key list
  val stats : 'a t -> int * int
end

module Make (Key : KEY) : CACHE with type key = Key.t = struct
  type key = Key.t

  type 'a node = {
    key : key;
    mutable value : 'a;
    mutable previous : 'a node option;
    mutable next : 'a node option;
  }

  module Table = Hashtbl.Make (struct
    type t = key

    let equal = Key.equal
    let hash = Key.hash
  end)

  type 'a t = {
    capacity : int;
    table : 'a node Table.t;
    mutable head : 'a node option;
    mutable tail : 'a node option;
    mutable hits : int;
    mutable misses : int;
  }

  let create capacity =
    if capacity <= 0 then invalid_arg "LRU capacity must be positive";
    {
      capacity;
      table = Table.create capacity;
      head = None;
      tail = None;
      hits = 0;
      misses = 0;
    }

  let capacity cache = cache.capacity
  let size cache = Table.length cache.table
  let stats cache = (cache.hits, cache.misses)

  let unlink cache node =
    (match node.previous with
    | Some previous -> previous.next <- node.next
    | None -> cache.head <- node.next);
    (match node.next with
    | Some next -> next.previous <- node.previous
    | None -> cache.tail <- node.previous);
    node.previous <- None;
    node.next <- None

  let push_front cache node =
    node.next <- cache.head;
    (match cache.head with Some head -> head.previous <- Some node | None -> ());
    cache.head <- Some node;
    if cache.tail = None then cache.tail <- Some node

  let touch cache node =
    unlink cache node;
    push_front cache node

  let evict cache =
    match cache.tail with
    | None -> ()
    | Some victim ->
        unlink cache victim;
        Table.remove cache.table victim.key

  let put cache key value =
    match Table.find_opt cache.table key with
    | Some node ->
        node.value <- value;
        touch cache node
    | None ->
        if size cache >= cache.capacity then evict cache;
        let node = { key; value; previous = None; next = None } in
        Table.add cache.table key node;
        push_front cache node

  let find_opt cache key =
    match Table.find_opt cache.table key with
    | Some node ->
        cache.hits <- cache.hits + 1;
        touch cache node;
        Some node.value
    | None ->
        cache.misses <- cache.misses + 1;
        None

  let remove cache key =
    match Table.find_opt cache.table key with
    | Some node ->
        unlink cache node;
        Table.remove cache.table key;
        true
    | None -> false

  let keys cache =
    let rec walk acc = function
      | None -> List.rev acc
      | Some node -> walk (node.key :: acc) node.next
    in
    walk [] cache.head
end

module StringKey = struct
  type t = string

  let equal = String.equal
  let hash = Hashtbl.hash
  let to_string key = key
end

module StringCache = Make (StringKey)

let hit_rate cache =
  let hits, misses = StringCache.stats cache in
  let total = hits + misses in
  if total = 0 then 0.0 else float_of_int hits /. float_of_int total

let () =
  let cache = StringCache.create 2 in
  StringCache.put cache "alpha" 1;
  StringCache.put cache "beta" 2;
  ignore (StringCache.find_opt cache "alpha");
  StringCache.put cache "gamma" 3;

  let remaining = StringCache.keys cache |> String.concat ", " in
  Printf.printf "kept: %s\n" remaining;
  Printf.printf "size %d of %d\n" (StringCache.size cache) (StringCache.capacity cache);
  Printf.printf "hit rate %.2f\n" (hit_rate cache)
