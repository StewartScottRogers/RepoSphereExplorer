# Warehouse.Inventory

Stock levels, reservations and reorder points, over whatever
`IStockRepository` you hand it.

## Using it

```csharp
var service = new InventoryService(repository, lowWaterMark: 5);

decimal total = await service.TotalValueAsync(token);
StockItem reserved = await service.ReserveAsync("SKU-1", count: 2, token);

foreach (var item in service.NeedingReorder(await repository.ListAsync(token)))
{
    Console.WriteLine($"reorder {item.Sku}");
}
```

## Notes

- `ReserveAsync` throws `OutOfStockException` rather than reserving what it
  can. A partial reservation nobody asked for is worse than a refusal.
- `Classify` compares against the low-water mark the service was built
  with, so two warehouses with different appetites use the same code.
- Every asynchronous method takes a `CancellationToken`. One that does not
  is a method that cannot be shut down.

## Building

```bash
dotnet restore
dotnet test
dotnet run --project src/Warehouse.Inventory
```

---

**This is a fixture.** It lives in `samples/csharp/` so the application has a
C# project to open, not just a C# file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
