# example/router

A small router in the shape PSR-15 describes: routes with path parameters,
middleware that wraps every request, and an exception rather than a blank
page when nothing matches.

## Using it

```php
$router = new Router();
$router->use(new TimingMiddleware());

$router->get('/orders/{id}', fn (Request $r) => new Response(200, load($r->parameter('id'))));

echo $router->dispatch(Request::fromGlobals())->body;
```

## Notes

- An unmatched path throws `RouteNotFoundException`. Returning an empty
  200 is how a routing bug reaches production disguised as a blank page.
- `Request` and `Response` are `final` and immutable. A handler that
  mutates the request its middleware already inspected is a handler nobody
  can reason about.
- Middleware is a list, run outermost first, so the order you add them in
  is the order they wrap.

## Developing

```bash
composer install
composer test
composer analyse
```

---

**This is a fixture.** It lives in `samples/php/` so the application has a
PHP project to open, not just a PHP file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
