using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace Warehouse.Inventory
{
    /// <summary>One item as the warehouse tracks it.</summary>
    public record StockItem(string Sku, string Name, int Quantity, decimal UnitPrice)
    {
        public decimal TotalValue => Quantity * UnitPrice;
    }

    public enum StockLevel
    {
        Out,
        Low,
        Healthy
    }

    public interface IStockRepository
    {
        Task<IReadOnlyList<StockItem>> ListAsync(CancellationToken token);
        Task SaveAsync(StockItem item, CancellationToken token);
    }

    public class OutOfStockException : Exception
    {
        public OutOfStockException(string sku)
            : base($"no stock left for {sku}")
        {
            Sku = sku;
        }

        public string Sku { get; }
    }

    public class InventoryService
    {
        private readonly IStockRepository _repository;
        private readonly int _lowWaterMark;

        public InventoryService(IStockRepository repository, int lowWaterMark = 5)
        {
            _repository = repository ?? throw new ArgumentNullException(nameof(repository));
            _lowWaterMark = lowWaterMark;
        }

        public async Task<decimal> TotalValueAsync(CancellationToken token = default)
        {
            var items = await _repository.ListAsync(token);
            return items.Sum(item => item.TotalValue);
        }

        public StockLevel Classify(StockItem item)
        {
            if (item.Quantity == 0)
            {
                return StockLevel.Out;
            }

            return item.Quantity <= _lowWaterMark ? StockLevel.Low : StockLevel.Healthy;
        }

        public async Task<StockItem> ReserveAsync(string sku, int count, CancellationToken token = default)
        {
            var items = await _repository.ListAsync(token);
            var item = items.FirstOrDefault(candidate => candidate.Sku == sku)
                ?? throw new KeyNotFoundException($"unknown sku {sku}");

            if (item.Quantity < count)
            {
                throw new OutOfStockException(sku);
            }

            var reserved = item with { Quantity = item.Quantity - count };
            await _repository.SaveAsync(reserved, token);
            return reserved;
        }

        public IEnumerable<StockItem> NeedingReorder(IEnumerable<StockItem> items) =>
            items.Where(item => Classify(item) != StockLevel.Healthy)
                 .OrderBy(item => item.Quantity);
    }

    public static class Program
    {
        public static void Main(string[] args)
        {
            var items = new List<StockItem>
            {
                new("WID-1", "Widget", 12, 4.50m),
                new("GAD-7", "Gadget", 2, 19.99m),
                new("SPR-3", "Sprocket", 0, 0.75m)
            };

            foreach (var item in items)
            {
                Console.WriteLine($"{item.Sku,-8} {item.Name,-10} {item.TotalValue,8:C}");
            }
        }
    }
}
