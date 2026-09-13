// The storage and the queue the readings collector writes to.

targetScope = 'resourceGroup'

@description('Where everything is created.')
param location string = resourceGroup().location

@description('Two to twenty-four lowercase letters and digits.')
@minLength(3)
@maxLength(24)
param storageName string

@allowed([
  'Standard_LRS'
  'Standard_GRS'
])
param storageSku string = 'Standard_LRS'

@description('How long a reading is kept before it is deleted.')
param retentionDays int = 30

@secure()
param collectorKey string

param tags object = {
  owner: 'the floor'
  system: 'csvstats'
}

var queueName = 'readings'
var containerName = 'archive'
var storageId = storage.id

resource storage 'Microsoft.Storage/storageAccounts@2023-05-01' = {
  name: storageName
  location: location
  sku: {
    name: storageSku
  }
  kind: 'StorageV2'
  tags: tags
  properties: {
    minimumTlsVersion: 'TLS1_2'
    allowBlobPublicAccess: false
  }
}

resource queueService 'Microsoft.Storage/storageAccounts/queueServices@2023-05-01' = {
  parent: storage
  name: 'default'
}

resource queue 'Microsoft.Storage/storageAccounts/queueServices/queues@2023-05-01' = {
  parent: queueService
  name: queueName
}

resource lifecycle 'Microsoft.Storage/storageAccounts/managementPolicies@2023-05-01' = {
  parent: storage
  name: 'default'
  properties: {
    policy: {
      rules: [
        {
          name: 'expire-archived-readings'
          enabled: true
          type: 'Lifecycle'
          definition: {
            filters: {
              blobTypes: [
                'blockBlob'
              ]
              prefixMatch: [
                '${containerName}/'
              ]
            }
            actions: {
              baseBlob: {
                delete: {
                  daysAfterModificationGreaterThan: retentionDays
                }
              }
            }
          }
        }
      ]
    }
  }
}

module alerts 'modules/alerts.bicep' = {
  name: 'readings-alerts'
  params: {
    location: location
    storageAccountId: storageId
    tags: tags
  }
}

module dashboard 'modules/dashboard.bicep' = {
  name: 'readings-dashboard'
  params: {
    location: location
    queueName: queueName
  }
}

output storageAccountId string = storage.id
output queueEndpoint string = storage.properties.primaryEndpoints.queue
output alertsResourceId string = alerts.outputs.actionGroupId
output collectorKeyLength int = length(collectorKey)
