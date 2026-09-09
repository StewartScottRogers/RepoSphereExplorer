using Warehouse.Inventory;
using Xunit;

namespace Warehouse.Inventory.Tests;

/// <summary>An in-memory repository, so the tests need no database.</summary>
internal sealed class FakeRepository : IStockRepository
{
    private readonly List<StockItem> _items;

    public FakeRepository(params StockItem[] items) => _items = [.. items];

    public List<StockItem> Saved { get; } = [];

    public Task<IReadOnlyList<StockItem>> ListAsync(CancellationToken token) =>
        Task.FromResult<IReadOnlyList<StockItem>>(_items);

    public Task SaveAsync(StockItem item, CancellationToken token)
    {
        Saved.Add(item);
        return Task.CompletedTask;
    }
}

public class InventoryServiceTests
{
    private static StockItem Item(string sku, int quantity, decimal price = 2.50m) =>
        new(sku, $"Item {sku}", quantity, price);

    [Fact]
    public async Task TotalValueAsync_SumsEveryItem()
    {
        var service = new InventoryService(new FakeRepository(Item("a", 2, 10m), Item("b", 3, 1m)));

        var total = await service.TotalValueAsync();

        Assert.Equal(23m, total);
    }

    [Theory]
    [InlineData(0, StockLevel.Out)]
    [InlineData(3, StockLevel.Low)]
    [InlineData(50, StockLevel.Healthy)]
    public void Classify_UsesTheLowWaterMark(int quantity, StockLevel expected)
    {
        var service = new InventoryService(new FakeRepository(), lowWaterMark: 5);

        Assert.Equal(expected, service.Classify(Item("a", quantity)));
    }

    [Fact]
    public async Task ReserveAsync_RefusesMoreThanIsThere()
    {
        var service = new InventoryService(new FakeRepository(Item("a", 1)));

        await Assert.ThrowsAsync<OutOfStockException>(() => service.ReserveAsync("a", 2));
    }

    [Fact]
    public void NeedingReorder_ReturnsOnlyWhatIsBelowTheMark()
    {
        var service = new InventoryService(new FakeRepository(), lowWaterMark: 5);

        var needing = service.NeedingReorder([Item("a", 1), Item("b", 90)]).ToList();

        Assert.Single(needing);
        Assert.Equal("a", needing[0].Sku);
    }
}
