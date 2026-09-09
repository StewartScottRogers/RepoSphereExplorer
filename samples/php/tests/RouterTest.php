<?php

declare(strict_types=1);

namespace App\Http\Tests;

use App\Http\Request;
use App\Http\Response;
use App\Http\RouteNotFoundException;
use App\Http\Router;
use App\Http\TimingMiddleware;
use PHPUnit\Framework\Attributes\Test;
use PHPUnit\Framework\TestCase;

final class RouterTest extends TestCase
{
    #[Test]
    public function itDispatchesToTheHandlerThatMatches(): void
    {
        $router = new Router();
        $router->get('/health', static fn (Request $request): Response => Response::text('ok'));

        $response = $router->dispatch(new Request('GET', '/health'));

        self::assertSame(200, $response->status);
        self::assertSame('ok', $response->body);
    }

    #[Test]
    public function itPassesPathParametersToTheHandler(): void
    {
        $router = new Router();
        $router->get(
            '/orders/{id}',
            static fn (Request $request): Response => Response::text($request->param('id') ?? '')
        );

        $response = $router->dispatch(new Request('GET', '/orders/4711'));

        self::assertSame('4711', $response->body);
    }

    #[Test]
    public function aMissingParameterFallsBackRatherThanFailing(): void
    {
        $request = new Request('GET', '/orders');

        self::assertSame('none', $request->param('id', 'none'));
    }

    #[Test]
    public function itThrowsRatherThanReturningAnEmptyResponseForAnUnknownPath(): void
    {
        $router = new Router();

        $this->expectException(RouteNotFoundException::class);

        $router->dispatch(new Request('GET', '/nothing-is-here'));
    }

    #[Test]
    public function middlewareSeesEveryRequest(): void
    {
        $router = new Router();
        $router->use(new TimingMiddleware());
        $router->get('/health', static fn (Request $request): Response => Response::text('ok'));

        $response = $router->dispatch(new Request('GET', '/health'));

        self::assertSame(200, $response->status);
    }
}
