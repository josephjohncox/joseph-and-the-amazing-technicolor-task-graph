declare module "node:http" {
  const http: {
    createServer(handler: (req: any, res: any) => void): {
      listen(port: number, host: string, callback?: () => void): void;
    };
  };
  export default http;
}

declare module "node:fs/promises" {
  export type Dirent = {
    name: string;
    isFile(): boolean;
  };
  export function appendFile(path: any, data: string | Uint8Array): Promise<void>;
  export function mkdir(path: any, options?: { recursive?: boolean }): Promise<string | undefined>;
  export function readFile(path: any): Promise<Uint8Array>;
  export function readFile(path: any, encoding: string): Promise<string>;
  export function readdir(path: any, options: { withFileTypes: true }): Promise<Dirent[]>;
}

declare module "node:crypto" {
  export function createHash(algorithm: string): {
    update(data: string | Uint8Array): {
      digest(encoding: "hex" | "base64" | "base64url"): string;
    };
  };
}

declare module "node:path" {
  export function dirname(path: string): string;
}

declare const process: {
  env: Record<string, string | undefined>;
};
