<?php

declare(strict_types=1);

/**
 * A small HTTP router: routes are registered with a method and a pattern
 * holding named placeholders, middleware wraps handlers, and a miss is an
 * exception rather than a silent 200.
 */

namespace App\Http;

use Closure;
use InvalidArgumentException;
use RuntimeException;

interface MiddlewareInterface
{
    public function handle(Request $request, Closure $next): Response;
}

final class Request
{
    public function __construct(
        public readonly string $method,
        public readonly string $path,
        public readonly array $query = [],
        public readonly array $params = [],
        public readonly ?string $body = null,
    ) {
    }

    public function withParams(array $params): self
    {
        return new self($this->method, $this->path, $this->query, $params, $this->body);
    }

    public function param(string $name, ?string $default = null): ?string
    {
        return $this->params[$name] ?? $default;
    }
}

final class Response
{
    private function __construct(
        public readonly int $status,
        public readonly string $body,
        public readonly array $headers = [],
    ) {
    }

    public static function text(string $body, int $status = 200): self
    {
        return new self($status, $body, ['Content-Type' => 'text/plain']);
    }

    public static function json(array $payload, int $status = 200): self
    {
        return new self($status, json_encode($payload, JSON_THROW_ON_ERROR), [
            'Content-Type' => 'application/json',
        ]);
    }

    public function isSuccessful(): bool
    {
        return $this->status >= 200 && $this->status < 300;
    }
}

class RouteNotFoundException extends RuntimeException
{
    public function __construct(public readonly string $method, public readonly string $path)
    {
        parent::__construct(sprintf('no route for %s %s', $method, $path));
    }
}

final class Route
{
    private string $regex;

    /** @var list<string> */
    private array $names = [];

    public function __construct(
        public readonly string $method,
        public readonly string $pattern,
        public readonly Closure $handler,
    ) {
        $this->regex = $this->compile($pattern);
    }

    private function compile(string $pattern): string
    {
        $regex = preg_replace_callback(
            '#\{(\w+)\}#',
            function (array $match): string {
                $this->names[] = $match[1];
                return '(?P<' . $match[1] . '>[^/]+)';
            },
            $pattern,
        );

        if ($regex === null) {
            throw new InvalidArgumentException("cannot compile route pattern {$pattern}");
        }

        return '#^' . $regex . '$#';
    }

    public function match(string $method, string $path): ?array
    {
        if ($method !== $this->method || preg_match($this->regex, $path, $matches) !== 1) {
            return null;
        }

        $params = [];
        foreach ($this->names as $name) {
            $params[$name] = $matches[$name];
        }

        return $params;
    }
}

final class Router
{
    /** @var list<Route> */
    private array $routes = [];

    /** @var list<MiddlewareInterface> */
    private array $middleware = [];

    public function get(string $pattern, Closure $handler): self
    {
        return $this->add('GET', $pattern, $handler);
    }

    public function post(string $pattern, Closure $handler): self
    {
        return $this->add('POST', $pattern, $handler);
    }

    public function add(string $method, string $pattern, Closure $handler): self
    {
        $this->routes[] = new Route(strtoupper($method), $pattern, $handler);
        return $this;
    }

    public function use(MiddlewareInterface $middleware): self
    {
        $this->middleware[] = $middleware;
        return $this;
    }

    public function dispatch(Request $request): Response
    {
        foreach ($this->routes as $route) {
            $params = $route->match($request->method, $request->path);
            if ($params === null) {
                continue;
            }

            $handler = fn (Request $matched): Response => ($route->handler)($matched);
            foreach (array_reverse($this->middleware) as $middleware) {
                $next = $handler;
                $handler = fn (Request $matched): Response => $middleware->handle($matched, $next);
            }

            return $handler($request->withParams($params));
        }

        throw new RouteNotFoundException($request->method, $request->path);
    }

    public function count(): int
    {
        return count($this->routes);
    }
}

final class TimingMiddleware implements MiddlewareInterface
{
    public function __construct(private array $timings = [])
    {
    }

    public function handle(Request $request, Closure $next): Response
    {
        $started = microtime(true);
        $response = $next($request);
        $this->timings[$request->path] = (microtime(true) - $started) * 1000;
        return $response;
    }

    public function slowest(): ?string
    {
        if ($this->timings === []) {
            return null;
        }
        arsort($this->timings);
        return array_key_first($this->timings);
    }
}

$router = (new Router())
    ->use(new TimingMiddleware())
    ->get('/health', fn (Request $request): Response => Response::text('ok'))
    ->get('/orders/{id}', fn (Request $request): Response => Response::json([
        'id' => $request->param('id'),
        'status' => 'shipped',
    ]))
    ->post('/orders', fn (Request $request): Response => Response::json(['created' => true], 201));

$response = $router->dispatch(new Request('GET', '/orders/42'));
echo $response->status, ' ', $response->body, PHP_EOL;
echo 'routes: ', $router->count(), PHP_EOL;
