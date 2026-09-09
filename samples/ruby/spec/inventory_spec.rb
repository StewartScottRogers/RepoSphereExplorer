# frozen_string_literal: true

RSpec.describe Warehouse::Inventory do
  subject(:inventory) { described_class.new([widget]) }

  let(:widget) do
    Warehouse::Item.new(
      sku: "WID-1",
      name: "Widget",
      price: Warehouse::Money.from_pounds(4.50),
      quantity: 3
    )
  end

  it "reports the version the gemspec releases" do
    expect(Warehouse::VERSION).to match(/\A\d+\.\d+\.\d+\z/)
  end

  it "finds an item it was built with" do
    expect(inventory["WID-1"].name).to eq("Widget")
  end

  it "raises UnknownSku rather than returning nil for something it has never seen" do
    expect { inventory["NOPE-9"] }.to raise_error(Warehouse::UnknownSku)
  end

  it "raises OutOfStock rather than handing over what it does not have" do
    expect { inventory.reserve("WID-1", 4) }.to raise_error(Warehouse::OutOfStock)
  end

  it "reserves what it does have" do
    inventory.reserve("WID-1", 2)

    expect(inventory["WID-1"].quantity).to eq(1)
  end

  it "totals the value of everything on the shelves" do
    expect(inventory.total_value.pennies).to eq(1350)
  end

  it "lists what has fallen below the threshold" do
    expect(inventory.low_stock(threshold: 5).map(&:sku)).to eq(["WID-1"])
  end

  describe Warehouse::Money do
    it "keeps money in pennies, so nothing is lost to rounding" do
      expect(Warehouse::Money.from_pounds(4.50).pennies).to eq(450)
    end

    it "adds without going through a float" do
      total = Warehouse::Money.from_pounds(0.10) + Warehouse::Money.from_pounds(0.20)

      expect(total.pennies).to eq(30)
    end
  end
end
