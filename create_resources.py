import boto3
import hmac
import hashlib
import base64
import datetime
import requests

def create_s3_bucket():
    print("Creating S3 bucket...")
    try:
        s3 = boto3.client('s3', 
                          endpoint_url='http://127.0.0.1:8333', 
                          aws_access_key_id='admin', 
                          aws_secret_access_key='password123',
                          region_name='us-east-1')
        s3.create_bucket(Bucket='test-bucket')
        print("✅ S3 Bucket 'test-bucket' created")
    except Exception as e:
        print(f"❌ S3 Error: {e}")

from azure.storage.blob import BlobServiceClient

def create_azure_container():
    print("Creating Azure container...")
    connection_string = "DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://localhost:10000/devstoreaccount1;"
    
    try:
        blob_service_client = BlobServiceClient.from_connection_string(connection_string)
        container_client = blob_service_client.get_container_client("test-container")
        
        try:
            container_client.create_container()
            print("✅ Azure Container 'test-container' created")
        except Exception as e:
            if "ContainerAlreadyExists" in str(e):
                print("ℹ️ Azure Container already exists")
            else:
                print(f"❌ Azure Error: {e}")
                
    except Exception as e:
        print(f"❌ Azure Script Error: {e}")

if __name__ == "__main__":
    create_s3_bucket()
    create_azure_container()
